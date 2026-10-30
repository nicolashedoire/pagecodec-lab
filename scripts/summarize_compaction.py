#!/usr/bin/env python3
"""Validate paired native compaction measurements and summarize every case.

Reads measurement JSON only; never runs the benchmark or changes measurements.
The primary timing is the native selection + copying + unmapping phase, not
whole-program time. Pilot results remain explicitly labelled throughout.
"""
import argparse
import hashlib
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / 'measurements/compaction'
POLICIES = ('fifo', 'benefit', 'efficiency')
SOURCES = ('household', 'covertype', 'rea_source', 'random_control')
REQUIRED = ('elapsed_ms', 'cpu_ms', 'bytes_moved',
            'released_bytes', 'target_bytes', 'max_reclaimable_bytes',
            'mapped_before', 'mapped_after', 'rss_before', 'rss_after',
            'peak_rss_bytes')
NULLABLE_TIMINGS = ('decision_ms', 'compaction_ms', 'reclaim_ms')


def number(value, label, *, positive=False, integer=False):
    if (isinstance(value, bool) or not isinstance(value, (int, float))
            or not math.isfinite(value) or value < 0 or (positive and value == 0)
            or (integer and value != int(value))):
        raise ValueError(f'Invalid {label}: {value!r}')
    return value


def require(condition, message):
    if not condition:
        raise ValueError(message)


def spread(values):
    values = list(values)
    return {'n': len(values), 'min': min(values),
            'median': statistics.median(values), 'max': max(values)}


def case_key(sample):
    return sample['case'], sample['layout'], float(sample['reclaim_fraction'])


def load_report(location):
    path = Path(location)
    if path.is_dir():
        path /= 'all-results.json'
    raw = path.read_bytes()
    report = json.loads(raw)
    require(report.get('schema_version') == 1, 'Unsupported compaction schema')
    config, manifest = report['frozen_config'], report['manifest']
    protocol = config['protocol']
    require(protocol == manifest['protocol'], 'Frozen protocol differs from manifest')
    require(isinstance(config.get('pilot'), bool), 'Missing explicit pilot/final flag')
    require(protocol['repetitions'] == 9, 'Expected frozen nine-trial protocol')
    require(set(protocol['policies']) == set(POLICIES), 'Unexpected policy set')
    require(len(protocol['policies']) == len(POLICIES), 'Duplicate policies')
    require({case['name'] for case in manifest['cases']} == set(SOURCES),
            'Unexpected corpus set')
    require(len(manifest['cases']) == len(SOURCES), 'Duplicate corpus names')
    require(protocol.get('none_control_repetitions') == 1,
            'Expected one no-compaction control per scenario')
    repetitions = 1 if config['pilot'] else protocol['repetitions']
    source_cases = {case['name']: case for case in manifest['cases']}
    expected = {(source, layout, float(fraction))
                for source in SOURCES for layout in protocol['layouts']
                for fraction in protocol['quotas_by_layout'][layout]}
    groups = defaultdict(lambda: defaultdict(dict))
    page_sizes = set()
    for sample in report['samples']:
        key = case_key(sample)
        label = f'{key}/{sample["policy"]}/{sample["trial"]}'
        require(key in expected, f'Unexpected scenario: {label}')
        policy, trial = sample['policy'], sample['trial']
        require(policy in (*POLICIES, 'none'), f'Unknown policy: {label}')
        number(trial, f'trial {label}', integer=True)
        require(trial < repetitions, f'Unexpected trial: {label}')
        require(policy != 'none' or trial == 0, f'Unexpected control trial: {label}')
        require(trial not in groups[key][policy], f'Duplicate sample: {label}')
        number(sample['seed'], f'seed {label}', integer=True)
        require(sample['seed'] == protocol['layout_seed_base'] + trial,
                f'Layout seed differs from frozen schedule: {label}')
        number(sample['order_position'], f'order position {label}', integer=True)
        number(sample['external_elapsed_ms'], f'external elapsed {label}', positive=True)
        measurement = sample['measurement']
        require(measurement.get('schema_version') == 1,
                f'Unsupported engine measurement schema: {label}')
        for field in ('policy', 'layout', 'seed', 'reclaim_fraction'):
            require(measurement[field] == sample[field], f'Engine/runner {field} mismatch: {label}')
        for field in REQUIRED:
            number(measurement[field], f'{field} {label}',
                   positive=field == 'elapsed_ms',
                   integer=field not in ('elapsed_ms', 'cpu_ms'))
        for field in NULLABLE_TIMINGS:
            require(field in measurement, f'Missing nullable phase field {field}: {label}')
            if measurement[field] is not None:
                number(measurement[field], f'{field} {label}')
        for field, value in measurement.items():
            if not isinstance(value, bool) and isinstance(value, (int, float)):
                number(value, f'{field} {label}')
        require(measurement.get('checksum_ok') is True, f'Failed payload verification: {label}')
        require(isinstance(measurement.get('checksum_before'), str)
                and len(measurement['checksum_before']) == 16
                and measurement['checksum_before'] == measurement.get('checksum_after'),
                f'Checksum values do not agree: {label}')
        source = source_cases[sample['case']]
        for field, expected_value in (
            ('copies', source['copies']), ('source_record_count', source['records']),
            ('total_record_count', source['copies'] * source['records']),
            ('logical_page_bytes', source['logical_page_bytes']),
            ('source_compressed_bytes', source['compressed_payload_bytes']),
            ('segment_bytes', protocol['segment_bytes']),
            ('conditioning_bytes', protocol['cache_scrub_mib'] * 2**20),
        ):
            require(measurement[field] == expected_value, f'Engine/manifest {field} mismatch: {label}')
        require(isinstance(measurement.get('target_met'), bool), f'Missing target status: {label}')
        released, target = measurement['released_bytes'], measurement['target_bytes']
        require(measurement['mapped_before'] - measurement['mapped_after'] == released,
                f'Mapping/release accounting mismatch: {label}')
        require(target <= measurement['max_reclaimable_bytes'] <= measurement['mapped_before'],
                f'Impossible reclaim capacity: {label}')
        require(released <= measurement['max_reclaimable_bytes'], f'Overclaimed reclaim: {label}')
        require(measurement['target_met'] == (released >= target), f'Incorrect target status: {label}')
        require(measurement.get('overshoot_bytes') == max(0, released-target),
                f'Incorrect recorded quota overshoot: {label}')
        if all(measurement[field] is not None for field in NULLABLE_TIMINGS):
            require(sum(measurement[field] for field in NULLABLE_TIMINGS)
                    <= measurement['elapsed_ms'] + 1e-6,
                    f'Phase times exceed native elapsed time: {label}')
        if policy != 'none':
            require(measurement['target_met'], f'Unmet compaction quota: {label}')
        else:
            require(released == 0 and measurement['bytes_moved'] == 0,
                    f'No-compaction control moved or released data: {label}')
        if target == 0:
            require(released == 0 and measurement['bytes_moved'] == 0,
                    f'Work performed despite zero reclaim target: {label}')
        for field in ('page_bytes', 'page_size', 'os_page_bytes'):
            if field in measurement:
                page = number(measurement[field], f'{field} {label}', positive=True, integer=True)
                page_sizes.add(page)
                for counter in ('mapped_before', 'mapped_after', 'released_bytes', 'target_bytes',
                                'max_reclaimable_bytes'):
                    require(measurement[counter] % page == 0,
                            f'Non-page-aligned {counter}: {label}')
                capacity_pages = measurement['max_reclaimable_bytes'] // page
                expected_target_pages = math.floor(capacity_pages * sample['reclaim_fraction'])
                if capacity_pages and not expected_target_pages:
                    expected_target_pages = 1
                require(target == expected_target_pages * page,
                        f'Target does not match frozen reclaim fraction: {label}')
        groups[key][policy][trial] = sample
    require(set(groups) == expected, 'Missing measurement scenarios')
    require(len(page_sizes) <= 1, 'OS page size changed between samples')
    full_reclaim_pairs = 0
    for key, policies in groups.items():
        require(set(policies) == {*POLICIES, 'none'}, f'Missing policy/control: {key}')
        for policy in POLICIES:
            require(set(policies[policy]) == set(range(repetitions)), f'Incomplete trials: {key}/{policy}')
        require(set(policies['none']) == {0}, f'Incomplete control: {key}')
        for trial in range(repetitions):
            paired = [policies[p][trial] for p in POLICIES]
            ordered = paired + ([policies['none'][0]] if trial == 0 else [])
            require(sorted(s['order_position'] for s in ordered) == list(range(len(ordered))),
                    f'Invalid shuffled order positions: {key}/{trial}')
            for field in ('seed',):
                require(len({s[field] for s in ordered}) == 1, f'Unpaired {field}: {key}/{trial}')
            for field in ('target_bytes', 'max_reclaimable_bytes', 'mapped_before',
                          'checksum_before', 'checksum_after', 'source_fnv1a64',
                          'live_record_count', 'deleted_record_count', 'live_payload_bytes',
                          'deleted_payload_bytes', 'common_empty_released_bytes',
                          'initial_tail_trimmed_bytes', 'conditioning_bytes'):
                require(len({s['measurement'][field] for s in ordered}) == 1,
                        f'Unpaired {field}: {key}/{trial}')
            if key[2] == 1.0:
                full_reclaim_pairs += 1
                for sample in paired:
                    m = sample['measurement']
                    require(m['released_bytes'] == m['target_bytes'] == m['max_reclaimable_bytes'],
                            f'Incomplete full reclaim: {key}/{trial}')
                require(len({s['measurement']['bytes_moved'] for s in paired}) == 1,
                        f'Full-reclaim policies performed different copy work: {key}/{trial}')
    validation = {
        'measurement_count': len(report['samples']), 'scenario_count': len(groups),
        'paired_trials_per_scenario': repetitions,
        'full_reclaim_paired_rounds_checked': full_reclaim_pairs,
        'page_alignment_checked': bool(page_sizes), 'page_bytes': next(iter(page_sizes), None),
        'paired_seeds_targets_and_initial_mapping': True,
        'checksums_passed': True, 'mapped_release_accounting_passed': True,
        'paired_live_handle_and_payload_checksums': True,
        'full_reclaim_same_release_and_bytes_moved': True,
        'unmeasured_detail_fields': [field for field in NULLABLE_TIMINGS
                                    if any(s['measurement'][field] is None for s in report['samples'])],
        'common_conditioning_bytes': protocol['cache_scrub_mib'] * 2**20,
    }
    return report, dict(groups), hashlib.sha256(raw).hexdigest(), validation


def sample_metrics(sample):
    m = sample['measurement']
    metrics = {k: v for k, v in m.items()
               if not isinstance(v, bool) and isinstance(v, (int, float)) and math.isfinite(v)}
    metrics['external_elapsed_ms'] = sample['external_elapsed_ms']
    metrics['rss_recovered_bytes'] = m['rss_before'] - m['rss_after']
    metrics['overshoot_bytes'] = max(0, m['released_bytes'] - m['target_bytes'])
    if m['target_bytes']:
        metrics['overshoot_pct_of_target'] = 100 * metrics['overshoot_bytes'] / m['target_bytes']
    if m['released_bytes']:
        metrics['rss_recovery_fraction_of_release'] = metrics['rss_recovered_bytes'] / m['released_bytes']
        metrics['moved_bytes_per_released_byte'] = m['bytes_moved'] / m['released_bytes']
    if m['decision_ms'] is not None:
        metrics['decision_pct_of_elapsed'] = 100 * m['decision_ms'] / m['elapsed_ms']
    return metrics


def aggregate(samples):
    metrics = [sample_metrics(s) for s in samples]
    fields = sorted(set.union(*(set(m) for m in metrics)))
    return {field: spread(m[field] for m in metrics if field in m) for field in fields}


def compare(pairs, field, *, external=False):
    values = [(b[field], a[field]) if external else
              (b['measurement'][field], a['measurement'][field]) for b, a in pairs]
    return {
        'paired_speedup_fifo_over_policy': spread(b / a for b, a in values),
        'paired_time_saved_pct': spread(100 * (1 - a / b) for b, a in values),
        'paired_time_saved_ms': spread(b - a for b, a in values),
        'faster_trials': sum(a < b for b, a in values),
        'tied_trials': sum(a == b for b, a in values),
        'slower_trials': sum(a > b for b, a in values),
    }


def build_summary(location):
    report, groups, source_hash, validation = load_report(location)
    rows, controls = [], []
    protocol = report['frozen_config']['protocol']
    for key in sorted(groups, key=lambda k: (SOURCES.index(k[0]), protocol['layouts'].index(k[1]), k[2])):
        policies = groups[key]
        identity = dict(zip(('case', 'layout', 'reclaim_fraction'), key))
        for policy in POLICIES:
            samples = [policies[policy][trial] for trial in sorted(policies[policy])]
            pairs = [(policies['fifo'][s['trial']], s) for s in samples]
            rows.append({**identity, 'policy': policy, 'trials': len(samples),
                         'all_targets_met': all(s['measurement']['target_met'] for s in samples),
                         'metrics': aggregate(samples),
                         'versus_fifo': {
                             'elapsed_ms': compare(pairs, 'elapsed_ms'),
                             'external_elapsed_ms': compare(pairs, 'external_elapsed_ms', external=True),
                         }})
        control = policies['none'][0]
        controls.append({**identity, 'policy': 'none', 'trials': 1,
                         'target_met': control['measurement']['target_met'],
                         'eligible_for_quota_time_comparison': control['measurement']['target_met'],
                         'metrics': aggregate([control])})
    return {
        'schema_version': 1, 'pilot': report['frozen_config']['pilot'],
        'status': 'pilot_only' if report['frozen_config']['pilot'] else 'final_frozen_protocol',
        'source_sha256': source_hash, 'validation': validation,
        'primary_timing': 'Native decision + copying + handle updates + unmapping; setup and validation excluded.',
        'statistics': 'Medians and observed min/max; paired ratios use matching layout seeds, not ratios of medians.',
        'limitations': [
            'Controlled user-space arena; real compressed payloads with synthetic deletion layouts.',
            'FIFO reproduces an eligibility guard, not the complete Apple kernel policy.',
            'Corpus copies repeat existing data; they are not independent source corpora.',
            'RSS recovery corroborates mappings and is not assumed to exactly equal bytes unmapped.',
            'No-compaction controls with unmet quotas are not competing time-to-quota measurements.',
            'Min/max ranges are observed variability, not confidence intervals or significance tests.',
            'Checksum and pairing validation relies on recorded engine fields; this script does not rerun the engine.',
            'Integrity is a noncryptographic checksum of stable handles, lengths and live compressed payloads; no decompression check.',
            'The common quota-capacity scan is reported separately from the native policy phase.',
            'Null detailed timings were not measured; they are never treated as zero or inferred by subtraction.',
            'Any cache-conditioning pass is common setup, not a guarantee of cold hardware caches.',
        ],
        'comparisons': rows, 'controls': controls,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=DEFAULT)
    parser.add_argument('--output', type=Path, help='Defaults to summary.json beside all-results.json')
    args = parser.parse_args()
    location = args.results.resolve()
    result = build_summary(location)
    output = args.output or ((location if location.is_dir() else location.parent) / 'summary.json')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
    label = 'PILOT ONLY' if result['pilot'] else 'FROZEN FINAL RUN'
    print(f'{label}: validated {result["validation"]["measurement_count"]} samples; wrote {output}')


if __name__ == '__main__':
    main()
