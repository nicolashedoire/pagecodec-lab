#!/usr/bin/env python3
"""Validate and flatten the published operation measurements without dependencies."""
import argparse
import csv
import json
import math
import statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / 'results/operations-2026-10-30'


def write_csv(path, rows):
    with path.open('w', newline='') as out:
        writer = csv.DictWriter(out, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def summarize(directory):
    report = json.loads((directory / 'all-results.json').read_text())
    timings, samples, memory, construction = [], [], [], []
    for case in report['experiments']:
        family, name = case['family'], case['case']['name']
        base = {'query': 'raw', 'recipe': 'materialized', 'strings': 'concat'}[family]
        local, value_map = [], {}
        rss = {r['method']: r['bytes'] for r in case['isolated_peak_rss']}

        def add_time(method, workload, values, unit):
            if len(values) != 9 or not all(math.isfinite(v) and v > 0 for v in values):
                raise ValueError('Expected nine positive finite timings')
            row = dict(family=family, corpus=name, method=method, workload=workload,
                       unit=unit, median=statistics.median(values), minimum=min(values),
                       maximum=max(values), baseline=base, speedup_vs_baseline=0.0,
                       median_paired_speedup=0.0, paired_faster_trials=0)
            local.append(row)
            value_map[(method, workload)] = values
            for trial, value in enumerate(values):
                samples.append(dict(family=family, corpus=name, method=method,
                                    workload=workload, unit=unit, trial=trial, value=value))

        for method in case['methods']:
            key = method.get('method', method.get('name'))
            if family == 'query':
                assert case['verified'] and case['representation_roundtrips_verified']
                logical = method['representation_bytes']
                assert logical == method['payload_bytes'] + method['metadata_bytes']
                owned = method['owned_working_bytes']
                definition = 'Conceptual store capacity plus scratch; shared main-run stores are not additive RSS'
                builds = [method['construction_ms']]
                for workload in case['workloads']:
                    result = next(m for m in workload['results'] if m['method'] == key)
                    assert result['verified']
                    values = result['samples_ms']
                    assert math.isclose(statistics.median(values), result['median_ms'], rel_tol=1e-8)
                    add_time(key, workload['kind'] + ':t=' + str(workload['threshold']), values, 'ms/schedule')
            elif family == 'recipe':
                assert case['checksums_verified']
                logical = method['memory']['source_logical_bytes'] + method['memory']['derived_stored_bytes']
                owned = method['memory']['retained_accounted_bytes']
                definition = 'Retained sources, derived representation, indexes, objects, scratch/cache and common schedule'
                builds = [s['wall_ms'] for s in method['construction_samples']]
                expected = {w['name']: w['expected_checksum'] for w in case['workloads']}
                for workload in method['workloads']:
                    assert workload['checksum'] == expected[workload['name']]
                    values = [s['wall_ms'] for s in workload['samples']]
                    assert math.isclose(statistics.median(values), workload['median_wall_ms'], rel_tol=1e-8)
                    add_time(key, workload['name'], values, 'ms/schedule')
            else:
                assert method['verified_all_objects']
                logical = method['logical_storage_bytes']
                assert logical == method['payload_bytes'] + method['index_bytes'] + method['block_metadata_bytes']
                owned = method['resident_copy_capacity_bytes']
                definition = 'Store capacity plus copy/decode workspace; excludes common trace and object headers'
                builds = method['construction_samples_ms']
                for access, contracts in method['workloads'].items():
                    for contract, workload in contracts.items():
                        add_time(key, access + '.' + contract, workload['ns_per_query_samples'], 'ns/query')
            memory.append(dict(family=family, corpus=name, method=key, logical_bytes=logical,
                               accounted_working_bytes=owned, accounting_definition=definition,
                               isolated_process_peak_rss_bytes=rss.get(key, ''),
                               rss_scope='Separate process high-water including input loading, construction, validation and workload'))
            construction.append(dict(family=family, corpus=name, method=key,
                                     median_ms=statistics.median(builds), minimum_ms=min(builds),
                                     maximum_ms=max(builds), sample_count=len(builds)))
        baselines = {r['workload']: r['median'] for r in local if r['method'] == base}
        for row in local:
            row['speedup_vs_baseline'] = baselines[row['workload']] / row['median']
            pairs = list(zip(value_map[(base, row['workload'])], value_map[(row['method'], row['workload'])]))
            row['median_paired_speedup'] = statistics.median(b / m for b, m in pairs)
            row['paired_faster_trials'] = sum(m < b for b, m in pairs)
        timings.extend(local)
    write_csv(directory / 'timings.csv', timings)
    write_csv(directory / 'timing-samples.csv', samples)
    write_csv(directory / 'memory.csv', memory)
    write_csv(directory / 'construction.csv', construction)
    overview = {'cases': len(report['experiments']), 'method_workload_comparisons': len(timings),
                'timing_samples': len(samples), 'methods': []}
    for m in memory:
        baseline_name = {'query': 'raw', 'recipe': 'materialized', 'strings': 'concat'}[m['family']]
        b = next(v for v in memory if v['family'] == m['family'] and v['corpus'] == m['corpus'] and v['method'] == baseline_name)
        rows = [r for r in timings if all(r[k] == m[k] for k in ('family', 'corpus', 'method'))]
        overview['methods'].append({k: m[k] for k in ('family', 'corpus', 'method')} | {
            'working_fraction_of_baseline': m['accounted_working_bytes'] / b['accounted_working_bytes'],
            'logical_fraction_of_baseline': m['logical_bytes'] / b['logical_bytes'],
            'speedup_min': min(r['speedup_vs_baseline'] for r in rows),
            'speedup_max': max(r['speedup_vs_baseline'] for r in rows),
            'workloads_at_least_10_percent_lower_time': sum(r['speedup_vs_baseline'] >= 1 / .9 for r in rows),
            'workload_count': len(rows)})
    (directory / 'summary.json').write_text(json.dumps(overview, indent=2) + '\n')
    print(f"Validated {len(timings)} method/workload results and {len(samples)} timed samples.")
    return timings, memory, construction


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=DEFAULT)
    summarize(parser.parse_args().results.resolve())
