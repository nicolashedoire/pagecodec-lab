#!/usr/bin/env python3
"""Validate and summarize the frozen adaptive-compaction experiment.

This script reads measurement JSON only. It does not benchmark, tune a policy,
or rerun the engine. Pilot data can be checked but cannot pass the final rule.
"""
import argparse
import hashlib
import json
import math
import statistics
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / 'results/compaction-adaptive-2026-10-30'
SOURCES = ('household_row', 'bike', 'cmu_pronunciations', 'random_control')
REAL_SOURCES = SOURCES[:3]
POLICIES = ('fifo', 'benefit', 'efficiency', 'tail_first', 'adaptive')
EXPECTED_RULE = ('Exploratory criterion: adaptive median paired phase time >=10% below FIFO '
                 'in scrambled 25% and 50% on at least 2 of 3 real sources, and no '
                 'real-source median regression >10% at scrambled 100%.')
QUOTAS = {'front': (.5,), 'tail': (.5,), 'random': (.25, .5, 1.),
          'mixed': (.25, .5, 1.), 'scrambled': (.25, .5, 1.),
          'rare_tail': (.25, .5, 1.)}
DETERMINISTIC = ('front', 'tail', 'mixed', 'rare_tail')
NULLABLE_TIMINGS = ('decision_ms', 'compaction_ms', 'reclaim_ms')
REQUIRED_NUMBERS = ('elapsed_ms', 'cpu_ms', 'released_bytes', 'bytes_moved',
                    'payload_bytes_moved', 'records_moved', 'segments_compacted',
                    'target_bytes', 'max_reclaimable_bytes', 'mapped_before',
                    'mapped_after', 'rss_before', 'rss_after', 'peak_rss_bytes',
                    'page_bytes', 'conditioning_bytes', 'conditioning_ms')
INITIAL_FIELDS = ('target_bytes', 'max_reclaimable_bytes', 'mapped_before',
                  'checksum_before', 'checksum_after', 'source_fnv1a64',
                  'live_record_count', 'deleted_record_count', 'live_payload_bytes',
                  'deleted_payload_bytes', 'common_empty_released_bytes',
                  'initial_tail_trimmed_bytes', 'conditioning_bytes')
FULL_WORK_FIELDS = ('released_bytes', 'bytes_moved', 'payload_bytes_moved',
                    'records_moved', 'segments_compacted')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value, label, *, positive=False, integer=False):
    require(not isinstance(value, bool) and isinstance(value, (int, float))
            and math.isfinite(value) and value >= 0 and (not positive or value > 0)
            and (not integer or value == int(value)), f'Invalid {label}: {value!r}')
    return value


def digest_shape(value, length):
    return (isinstance(value, str) and len(value) == length
            and all(c in '0123456789abcdef' for c in value))


def spread(values):
    values = list(values)
    if not values:
        return None
    return {'n': len(values), 'min': min(values),
            'median': statistics.median(values), 'max': max(values)}


def load_report(location):
    path = Path(location)
    if path.is_dir():
        path /= 'all-results.json'
    raw = path.read_bytes()
    report = json.loads(raw)
    require(report.get('schema_version') == 1, 'Unsupported measurement schema')
    config, manifest = report['frozen_config'], report['manifest']
    protocol = config['protocol']
    require(protocol == manifest['protocol'], 'Frozen protocol differs from embedded manifest')
    require(isinstance(config.get('pilot'), bool), 'An explicit pilot/final flag is required')
    require(tuple(protocol['policies']) == POLICIES, 'Unexpected frozen policy set/order')
    require(tuple(protocol['layouts']) == tuple(QUOTAS), 'Unexpected layout set/order')
    require({k: tuple(v) for k, v in protocol['quotas_by_layout'].items()} == QUOTAS,
            'Unexpected reclaim fractions')
    require(protocol['repetitions'] == 5 and protocol['control_repetitions'] == 1,
            'Expected five paired trials and one no-compaction control')
    require(protocol['success_rule'] == EXPECTED_RULE,
            'The recorded success rule differs from the rule implemented by this summarizer')
    require(digest_shape(config['binary_sha256'], 64), 'Missing recorded executable SHA-256')
    require(config['sources_sha256'] and all(digest_shape(v, 64) for v in config['sources_sha256'].values()),
            'Malformed recorded code hashes')
    sources = {case['name']: case for case in manifest['cases']}
    require(set(sources) == set(SOURCES) and len(manifest['cases']) == len(SOURCES),
            'Missing, duplicate, or unexpected source cases')
    for source in sources.values():
        for field in ('source', 'packed', 'upstream_manifest'):
            require(digest_shape(source[field]['sha256'], 64), f'Malformed {field} SHA-256')
    repetitions = 1 if config['pilot'] else 5
    expected = {(source, layout, fraction) for source in SOURCES
                for layout, quotas in QUOTAS.items() for fraction in quotas}
    require(len(expected) == 56, 'Internal protocol scenario mismatch')
    expected_count = len(expected) * (len(POLICIES) * repetitions + 1)
    require(len(report['samples']) == expected_count,
            f'Expected {expected_count} samples, got {len(report["samples"])}')
    groups = defaultdict(lambda: defaultdict(dict))
    fnv_by_source, page_sizes = defaultdict(set), set()
    for sample in report['samples']:
        key = sample['case'], sample['layout'], float(sample['reclaim_fraction'])
        policy, trial = sample['policy'], sample['trial']
        label = f'{key}/{policy}/{trial}'
        require(key in expected, f'Unexpected case: {label}')
        require(policy in (*POLICIES, 'none'), f'Unexpected policy: {label}')
        number(trial, f'trial {label}', integer=True)
        require(trial < repetitions and (policy != 'none' or trial == 0), f'Unexpected trial: {label}')
        require(trial not in groups[key][policy], f'Duplicate sample: {label}')
        number(sample['seed'], f'seed {label}', integer=True)
        require(sample['seed'] == protocol['layout_seed_base'] + trial,
                f'Seed differs from frozen schedule: {label}')
        number(sample['order_position'], f'order position {label}', integer=True)
        number(sample['external_elapsed_ms'], f'external time {label}', positive=True)
        m = sample['measurement']
        require(m.get('schema_version') == 1, f'Unsupported engine schema: {label}')
        for field in ('policy', 'layout', 'seed', 'reclaim_fraction'):
            require(m[field] == sample[field], f'Engine/runner {field} mismatch: {label}')
        for field in REQUIRED_NUMBERS:
            number(m[field], f'{field} {label}', positive=field in ('elapsed_ms', 'page_bytes'),
                   integer=not field.endswith('_ms'))
        for field in NULLABLE_TIMINGS:
            require(field in m, f'Missing nullable phase field {field}: {label}')
            if m[field] is not None:
                number(m[field], f'{field} {label}')
                require(m[field] <= m['elapsed_ms'] + 1e-6,
                        f'Measured {field} exceeds native elapsed time: {label}')
        for field, value in m.items():
            if not isinstance(value, bool) and isinstance(value, (int, float)):
                number(value, f'{field} {label}')
        require(m['checksum_ok'] is True and digest_shape(m['checksum_before'], 16)
                and m['checksum_before'] == m['checksum_after'], f'Payload/handle checksum failed: {label}')
        require(digest_shape(m['source_fnv1a64'], 16), f'Malformed source FNV: {label}')
        fnv_by_source[key[0]].add(m['source_fnv1a64'])
        source = sources[key[0]]
        for field, value in (
            ('copies', source['copies']), ('source_record_count', source['records']),
            ('total_record_count', source['records'] * source['copies']),
            ('source_logical_bytes', source['source']['bytes']),
            ('source_compressed_bytes', source['compressed_payload_bytes']),
            ('segment_bytes', protocol['segment_bytes']),
            ('conditioning_bytes', protocol['cache_scrub_mib'] * 2**20),
        ):
            require(m[field] == value, f'Engine/manifest {field} mismatch: {label}')
        released, target, maximum, page = (m[f] for f in
                                          ('released_bytes', 'target_bytes', 'max_reclaimable_bytes', 'page_bytes'))
        page_sizes.add(page)
        require(m['mapped_before'] - m['mapped_after'] == released, f'Mapping/release mismatch: {label}')
        require(target <= maximum <= m['mapped_before'] and released <= maximum,
                f'Impossible reclaim accounting: {label}')
        for field in ('mapped_before', 'mapped_after', 'released_bytes', 'target_bytes', 'max_reclaimable_bytes'):
            require(m[field] % page == 0, f'Non-page-aligned {field}: {label}')
        pages = maximum // page
        target_pages = max(1, math.floor(pages * key[2])) if pages else 0
        require(target == target_pages * page, f'Target differs from specified fraction: {label}')
        require(isinstance(m['target_met'], bool) and m['target_met'] == (released >= target),
                f'Incorrect target status: {label}')
        require(m['overshoot_bytes'] == max(0, released-target), f'Incorrect overshoot: {label}')
        require(m['payload_bytes_moved'] <= m['bytes_moved'], f'Payload exceeds aligned movement: {label}')
        if policy == 'none':
            require(released == 0 and m['bytes_moved'] == 0 and m['records_moved'] == 0,
                    f'No-compaction control performed compaction: {label}')
        else:
            require(m['target_met'], f'Compaction quota unmet: {label}')
        if target == 0:
            require(released == 0 and m['bytes_moved'] == 0 and m['records_moved'] == 0,
                    f'Compaction work despite zero target: {label}')
        if policy == 'adaptive':
            selected = 'tail_first' if target < maximum else 'fifo'
            require(m['selected_policy'] == selected, f'Adaptive branch disagrees with capacity: {label}')
            if key[2] < 1 and target > 0:
                require(selected == 'tail_first', f'Partial positive quota unexpectedly equals capacity: {label}')
        else:
            require(m['selected_policy'] == policy, f'Wrong reported selected policy: {label}')
        groups[key][policy][trial] = sample
    require(set(groups) == expected, 'Missing measurement scenarios')
    require(len(page_sizes) == 1, 'OS page size changed across samples')
    require(all(len(values) == 1 for values in fnv_by_source.values()),
            'Source FNV changed between policies, layouts, quotas or trials')
    full_rounds, initial_by_layout_trial = 0, {}
    for key, policies in groups.items():
        require(set(policies) == {*POLICIES, 'none'}, f'Missing policy/control: {key}')
        for policy in POLICIES:
            require(set(policies[policy]) == set(range(repetitions)), f'Incomplete paired trials: {key}/{policy}')
        require(set(policies['none']) == {0}, f'Invalid control set: {key}')
        for trial in range(repetitions):
            competitors = [policies[p][trial] for p in POLICIES]
            paired = competitors + ([policies['none'][0]] if trial == 0 else [])
            require(sorted(s['order_position'] for s in paired) == list(range(len(paired))),
                    f'Invalid shuffled order positions: {key}/{trial}')
            require(len({s['seed'] for s in paired}) == 1, f'Unpaired seeds: {key}/{trial}')
            for field in INITIAL_FIELDS:
                require(len({s['measurement'][field] for s in paired}) == 1,
                        f'Unpaired initial state {field}: {key}/{trial}')
            initial_key = key[0], key[1], trial
            initial = tuple(competitors[0]['measurement'][field] for field in INITIAL_FIELDS if field != 'target_bytes')
            if initial_key in initial_by_layout_trial:
                require(initial == initial_by_layout_trial[initial_key],
                        f'Changing quota changed initial arena: {initial_key}')
            initial_by_layout_trial[initial_key] = initial
            if key[2] == 1.:
                full_rounds += 1
                for sample in competitors:
                    m = sample['measurement']
                    require(m['released_bytes'] == m['target_bytes'] == m['max_reclaimable_bytes'],
                            f'Full quota did not reclaim all available pages: {key}/{trial}')
                for field in FULL_WORK_FIELDS:
                    require(len({s['measurement'][field] for s in competitors}) == 1,
                            f'Full-quota work differs for {field}: {key}/{trial}')
    validation = {
        'measurement_count': len(report['samples']), 'scenario_count': len(groups),
        'paired_trials_per_scenario': repetitions, 'control_count': len(groups),
        'full_reclaim_paired_rounds_checked': full_rounds,
        'page_bytes': next(iter(page_sizes)), 'common_conditioning_bytes': protocol['cache_scrub_mib'] * 2**20,
        'paired_seeds_targets_mapping_checksums': True, 'quota_independent_initial_arenas': True,
        'source_fnv_constant_per_source': True,
        'source_fnv_values': {source: next(iter(fnv_by_source[source])) for source in SOURCES},
        'source_fnv_is_independent_source_oracle': False,
        'full_reclaim_equal_work_fields': list(FULL_WORK_FIELDS),
        'adaptive_branch_matches_capacity': True,
        'unmeasured_detail_fields': [field for field in NULLABLE_TIMINGS
                                    if any(s['measurement'][field] is None for s in report['samples'])],
    }
    return report, dict(groups), hashlib.sha256(raw).hexdigest(), validation


def sample_metrics(sample):
    m = sample['measurement']
    metrics = {k: v for k, v in m.items()
               if not isinstance(v, bool) and isinstance(v, (int, float)) and math.isfinite(v)}
    metrics['external_elapsed_ms'] = sample['external_elapsed_ms']
    metrics['rss_recovered_bytes'] = m['rss_before'] - m['rss_after']
    for field in ('released_bytes', 'target_bytes', 'overshoot_bytes'):
        metrics[field.replace('_bytes', '_pages')] = m[field] / m['page_bytes']
    if m['target_bytes']:
        metrics['overshoot_pct_of_target'] = 100 * m['overshoot_bytes'] / m['target_bytes']
    if m['released_bytes']:
        metrics['rss_recovery_fraction_of_release'] = metrics['rss_recovered_bytes'] / m['released_bytes']
        metrics['moved_bytes_per_released_byte'] = m['bytes_moved'] / m['released_bytes']
    if m['decision_ms'] is not None:
        metrics['decision_pct_of_elapsed'] = 100 * m['decision_ms'] / m['elapsed_ms']
    return metrics


def aggregate(samples):
    metrics = [sample_metrics(sample) for sample in samples]
    fields = sorted(set.union(*(set(m) for m in metrics)))
    return {field: spread(m[field] for m in metrics if field in m) for field in fields}


def compare(pairs, field):
    values = [(b[field], a[field]) if field == 'external_elapsed_ms' else
              (b['measurement'][field], a['measurement'][field]) for b, a in pairs]
    return {
        'paired_speedup_baseline_over_policy': spread(b/a for b, a in values if a > 0),
        'paired_time_saved_pct': spread(100*(1-a/b) for b, a in values if b > 0),
        'paired_time_saved_ms': spread(b-a for b, a in values),
        'faster_trials': sum(a < b for b, a in values),
        'tied_trials': sum(a == b for b, a in values),
        'slower_trials': sum(a > b for b, a in values),
        'zero_baseline_trials': sum(b == 0 for b, a in values),
        'zero_policy_trials': sum(a == 0 for b, a in values),
    }


def criterion(rows, pilot, rule_text):
    index = {(r['case'], r['layout'], r['reclaim_fraction'], r['policy']): r for r in rows}
    details, passed_sources = [], []
    for source in REAL_SOURCES:
        savings = {quota: index[source, 'scrambled', quota, 'adaptive']['versus']['fifo']
                   ['elapsed_ms']['paired_time_saved_pct']['median'] for quota in (.25, .5, 1.)}
        both = savings[.25] >= 10 and savings[.5] >= 10
        guard = savings[1.] >= -10
        if both:
            passed_sources.append(source)
        details.append({'case': source, 'scrambled25_time_saved_pct': savings[.25],
                        'scrambled50_time_saved_pct': savings[.5],
                        'scrambled100_time_saved_pct': savings[1.],
                        'gain_at_both_partial_quotas': both, 'full_reclaim_regression_guard_passed': guard})
    full_guard = all(row['full_reclaim_regression_guard_passed'] for row in details)
    return {
        'text': rule_text, 'method': 'adaptive', 'baseline': 'fifo', 'metric': 'median paired native time savings',
        'threshold_pct': 10, 'minimum_sources': 2, 'evaluated': not pilot,
        'passed': None if pilot else len(passed_sources) >= 2 and full_guard,
        'passing_same_sources': passed_sources,
        'full_reclaim_guard_passed': None if pilot else full_guard,
        'per_real_source': details,
        'interpretation': 'Predeclared exploratory rule for these fixtures; not a significance test or a generalization guarantee.',
    }


def build_summary(location):
    report, groups, source_hash, validation = load_report(location)
    rows, controls = [], []
    for key in sorted(groups, key=lambda k: (SOURCES.index(k[0]), list(QUOTAS).index(k[1]), k[2])):
        policies = groups[key]
        identity = dict(zip(('case', 'layout', 'reclaim_fraction'), key))
        for policy in POLICIES:
            samples = [policies[policy][trial] for trial in sorted(policies[policy])]
            contrasts = {}
            for baseline in ('fifo', 'efficiency'):
                pairs = [(policies[baseline][sample['trial']], sample) for sample in samples]
                contrasts[baseline] = {field: compare(pairs, field)
                                       for field in ('elapsed_ms', 'cpu_ms', 'external_elapsed_ms')}
            rows.append({**identity, 'policy': policy, 'trials': len(samples),
                         'all_targets_met': all(s['measurement']['target_met'] for s in samples),
                         'selected_policies': dict(Counter(s['measurement']['selected_policy'] for s in samples)),
                         'metrics': aggregate(samples), 'versus': contrasts})
        sample = policies['none'][0]
        controls.append({**identity, 'policy': 'none', 'trials': 1,
                         'target_met': sample['measurement']['target_met'],
                         'eligible_for_quota_time_comparison': sample['measurement']['target_met'],
                         'metrics': aggregate([sample])})
    pilot = report['frozen_config']['pilot']
    return {
        'schema_version': 1, 'pilot': pilot,
        'status': 'pilot_only' if pilot else 'final_frozen_protocol',
        'source_sha256': source_hash, 'validation': validation,
        'environment': {field: report['frozen_config'].get(field)
                        for field in ('platform', 'machine', 'compiler', 'recorded_utc')},
        'sources': report['manifest']['cases'],
        'primary_timing': report['manifest']['protocol']['primary_timing'],
        'statistics': 'Medians and observed min/max. Percentages and speedups are computed per paired trial before aggregation.',
        'layout_variability': {'seed_varies_deletions': ['random', 'scrambled'],
                               'deterministic_repeated_layouts': list(DETERMINISTIC)},
        'success_criterion': criterion(rows, pilot, report['manifest']['protocol']['success_rule']),
        'limitations': [
            'Controlled native user-space arena, not a modified macOS compressor or a complete application benchmark.',
            'Household row layout reuses the Household source; Bike and CMU are new to native compaction, not unseen elsewhere in the repository.',
            'Copies repeat a corpus; five repetitions and many segments do not create independent real-world sources.',
            'Random and scrambled vary by seed. Front, tail, mixed and rare_tail repeat deterministic layouts.',
            'Rare_tail places a tail-deletion segment at indices 15 modulo 16; it is not a tail cluster at the end of the arena.',
            'FNV constancy is checked across recorded samples, not against an independent implementation of the source parser.',
            'The runner checked input SHA-256 values before execution; this summary does not claim an end-of-run rehash of input files.',
            'The summary trusts recorded engine integrity fields; it does not independently rerun compaction or decompression.',
            'Setup, common quota scan, checksum verification and cache conditioning are outside the primary native phase.',
            'Adaptive capacity recomputation and tail-first searches are inside the native phase. Null subphase timings were not measured.',
            'Shared cache conditioning does not guarantee cold hardware caches. RSS corroborates mappings but need not equal released bytes.',
            'No-compaction controls that miss the quota are not competing time-to-quota results.',
            'Observed min/max and win counts are descriptive, not confidence intervals or guarantees.',
        ],
        'comparisons': rows, 'controls': controls,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=DEFAULT)
    parser.add_argument('--output', type=Path, help='Defaults to summary.json beside the input JSON')
    args = parser.parse_args()
    location = args.results.resolve()
    summary = build_summary(location)
    output = args.output or ((location if location.is_dir() else location.parent) / 'summary.json')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, indent=2, allow_nan=False) + '\n')
    label = 'PILOT ONLY' if summary['pilot'] else 'FROZEN FINAL RUN'
    verdict = 'not evaluated' if summary['pilot'] else str(summary['success_criterion']['passed'])
    print(f'{label}: validated {summary["validation"]["measurement_count"]} samples; exploratory rule = {verdict}; wrote {output}')


if __name__ == '__main__':
    main()
