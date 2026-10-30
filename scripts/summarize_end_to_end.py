#!/usr/bin/env python3
"""Validate fresh-process report measurements and publish all comparisons."""
import argparse
import csv
import json
import math
import statistics as st
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT/'results/end-to-end-2026-10-30'


def write_csv(path, rows):
    with path.open('w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)


def summarize(directory):
    report = json.loads((directory/'all-results.json').read_text())
    methods = report['frozen_config']['protocol']['methods']
    summaries, processes = [], []
    assert len(report['cases']) == 12
    for case in report['cases']:
        assert len(case['samples']) == 45
        assert case['plan']['unique_queries'] == case['plan']['query_count']
        grouped = {m: sorted([s for s in case['samples'] if s['method']==m], key=lambda s:s['trial']) for m in methods}
        expected_hash = case['plan']['expected']['sha256']
        for m, samples in grouped.items():
            assert [s['trial'] for s in samples] == list(range(9))
            assert all(s['answers_sha256']==expected_hash and s['independent_answers_verified'] for s in samples)
            assert all(math.isfinite(s['external_elapsed_ms']) and s['external_elapsed_ms']>0 and s['peak_rss_bytes']>0 for s in samples)
        for trial in range(9):
            assert sorted(grouped[m][trial]['order_position'] for m in methods)==list(range(5))
        raw = grouped['raw']
        base_time = st.median(s['external_elapsed_ms'] for s in raw)
        base_rss = st.median(s['peak_rss_bytes'] for s in raw)
        for method, samples in grouped.items():
            times = [s['external_elapsed_ms'] for s in samples]
            peaks = [s['peak_rss_bytes'] for s in samples]
            tm, rm = st.median(times), st.median(peaks)
            summaries.append({'corpus':case['case'],'queries':case['plan']['query_count'],'method':method,
                'median_external_ms':tm,'min_external_ms':min(times),'max_external_ms':max(times),
                'median_peak_rss_bytes':rm,'min_peak_rss_bytes':min(peaks),'max_peak_rss_bytes':max(peaks),
                'speedup_ratio_of_medians':base_time/tm,'rss_fraction_of_raw_median':rm/base_rss,
                'median_paired_speedup':st.median(b['external_elapsed_ms']/s['external_elapsed_ms'] for b,s in zip(raw,samples)),
                'paired_faster_trials':sum(s['external_elapsed_ms']<b['external_elapsed_ms'] for b,s in zip(raw,samples)),
                'paired_lower_rss_trials':sum(s['peak_rss_bytes']<b['peak_rss_bytes'] for b,s in zip(raw,samples)),
                'paired_both_10pct_trials':sum(s['external_elapsed_ms']<=.9*b['external_elapsed_ms'] and s['peak_rss_bytes']<=.9*b['peak_rss_bytes'] for b,s in zip(raw,samples)),
                'median_both_10pct_pass':tm<=.9*base_time and rm<=.9*base_rss,
                'median_parse_and_build_ms':st.median(s['application']['timing_ms']['store_setup_source_parse_and_build'] for s in samples),
                'median_post_ingestion_query_ms':st.median(s['application']['timing_ms']['post_ingestion_queries'] for s in samples)})
            for sample in samples:
                processes.append({'corpus':case['case'],'queries':case['plan']['query_count'],'method':method,
                    'trial':sample['trial'],'order_position':sample['order_position'],
                    'external_elapsed_ms':sample['external_elapsed_ms'],'peak_rss_bytes':sample['peak_rss_bytes'],
                    'answers_sha256':sample['answers_sha256'],'independent_answers_verified':True})
    verdicts = []
    for method in methods:
        wins = [r['corpus'] for r in summaries if r['method']==method and r['queries']==4096 and r['median_both_10pct_pass']]
        verdicts.append({'method':method,'passing_sources_at_4096':wins,'passes_predeclared_two_source_rule':len(wins)>=2})
    write_csv(directory/'summary.csv', summaries)
    write_csv(directory/'process-samples.csv', processes)
    (directory/'verdicts.json').write_text(json.dumps({'rule':report['frozen_config']['protocol']['success_rule'],
        'measurement_count':len(processes),'comparisons':len(summaries),'verdicts':verdicts},indent=2)+'\n')
    print(f'Validated {len(processes)} process measurements and {len(summaries)} method/case comparisons.')
    for verdict in verdicts:
        print(verdict)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--results',type=Path,default=DEFAULT)
    summarize(p.parse_args().results.resolve())
