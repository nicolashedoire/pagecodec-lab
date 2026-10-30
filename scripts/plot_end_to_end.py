#!/usr/bin/env python3
"""Plot complete-report elapsed time and measured process peak RSS.

This script only reads the committed measurement JSON. It does not benchmark,
download data or modify measurements. Requires requirements-figures.txt.
"""
import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path

import matplotlib

matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.ticker import FixedLocator, FuncFormatter, LogLocator, MaxNLocator, NullLocator


ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / 'results/end-to-end-2026-10-30'
IVORY, NAVY, TEAL, COBALT = '#FAF8F2', '#142C43', '#008A80', '#335DCB'
MUTED, GRID = '#637787', '#DBE1E5'
COUNTS = (1, 32, 512, 4096)
SOURCES = ('household', 'covertype', 'bike')
SOURCE_LABELS = {'household': 'Household power', 'covertype': 'Covertype', 'bike': 'Bike sharing'}
METHODS = ('raw', 'bitplane', 'bitplane_adaptive', 'sorted_blocks', 'stream_sweep')
LABELS = {
    'raw': 'Raw + block aggregates',
    'bitplane': 'Progressive bitplanes',
    'bitplane_adaptive': 'Adaptive bitplanes',
    'sorted_blocks': 'Sorted-block index',
    'stream_sweep': 'Streaming report sweep',
}
COLORS = {
    'raw': '#7F8B94', 'bitplane': COBALT, 'bitplane_adaptive': '#9266AC',
    'sorted_blocks': '#C18B37', 'stream_sweep': TEAL,
}
MARKERS = {'raw': 'o', 'bitplane': 's', 'bitplane_adaptive': '^', 'sorted_blocks': 'D', 'stream_sweep': 'P'}
STYLES = {'raw': '-', 'bitplane': '-', 'bitplane_adaptive': '--', 'sorted_blocks': '-.', 'stream_sweep': '-'}


def positive(value, field):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
        raise ValueError(f'Expected positive finite {field}, got {value!r}')
    return float(value)


def load(directory):
    path = directory / 'all-results.json'
    data = path.read_bytes()
    report = json.loads(data)
    if report.get('schema_version') != 1:
        raise ValueError('Unknown result schema')
    protocol = report['frozen_config']['protocol']
    if protocol['repetitions'] != 9 or tuple(protocol['query_counts']) != COUNTS:
        raise ValueError('Expected the frozen nine-trial, four-load-level protocol')
    if set(protocol['methods']) != set(METHODS):
        raise ValueError('Method set differs from the frozen comparison')
    expected_keys = {(source, count) for source in SOURCES for count in COUNTS}
    cases, stats = {}, {}
    for case in report['cases']:
        key = case['case'], case['plan']['query_count']
        if key not in expected_keys or key in cases:
            raise ValueError(f'Unexpected or duplicate case: {key}')
        if case['plan']['unique_queries'] != key[1]:
            raise ValueError(f'Duplicate queries in workload: {key}')
        samples = case['samples']
        if len(samples) != len(METHODS) * 9:
            raise ValueError(f'Expected 45 fresh processes: {key}')
        answers = {sample['answers_sha256'] for sample in samples}
        if len(answers) != 1 or next(iter(answers)) != case['plan']['expected']['sha256']:
            raise ValueError(f'Answer digests do not match the independent oracle: {key}')
        for trial in range(9):
            group = [sample for sample in samples if sample['trial'] == trial]
            if {sample['method'] for sample in group} != set(METHODS) or len(group) != 5:
                raise ValueError(f'Incomplete paired round: {key}/{trial}')
            if sorted(sample['order_position'] for sample in group) != list(range(5)):
                raise ValueError(f'Invalid paired method positions: {key}/{trial}')
        for method in METHODS:
            group = [sample for sample in samples if sample['method'] == method]
            if len(group) != 9 or {sample['trial'] for sample in group} != set(range(9)):
                raise ValueError(f'Invalid trial set: {key}/{method}')
            for sample in group:
                if not sample['independent_answers_verified']:
                    raise ValueError(f'Unverified answers: {key}/{method}')
                application = sample['application']
                if application['evaluation_rows'] != case['rows'] or application['query_count'] != key[1]:
                    raise ValueError(f'Inconsistent application dimensions: {key}/{method}')
            for field in ('external_elapsed_ms', 'peak_rss_bytes'):
                values = [positive(sample[field], field) for sample in group]
                stats[key + (method, field)] = (min(values), statistics.median(values), max(values))
        cases[key] = case
    if set(cases) != expected_keys:
        raise ValueError('Missing measured cases')
    return report, cases, stats, hashlib.sha256(data).hexdigest()


def compact(value):
    if value >= 1000:
        return f'{value:,.0f}'
    if value >= 10:
        return f'{value:.0f}'
    if value >= 1:
        return f'{value:.1f}'.rstrip('0').rstrip('.')
    return f'{value:.2g}'


def axis_style(axis, show_x):
    axis.set_facecolor(IVORY)
    axis.set_xscale('log', base=2)
    axis.set_xlim(.68, 6000)
    axis.xaxis.set_major_locator(FixedLocator(COUNTS))
    axis.xaxis.set_major_formatter(FuncFormatter(lambda value, _: f'{value:,.0f}'))
    axis.xaxis.set_minor_locator(NullLocator())
    axis.tick_params(axis='both', length=0, labelsize=11, colors=MUTED, pad=8)
    for spine in axis.spines.values():
        spine.set_visible(False)
    axis.grid(which='major', axis='both', color=GRID, linewidth=.7)
    axis.set_axisbelow(True)
    if show_x:
        axis.set_xlabel('Unique queries in the report · logarithmic spacing', fontsize=11, labelpad=12)
    else:
        axis.tick_params(axis='x', labelbottom=False)


def render(directory, output):
    report, cases, stats, source_hash = load(directory)
    plt.rcParams.update({
        'font.family': 'DejaVu Sans', 'font.size': 12,
        'figure.facecolor': IVORY, 'savefig.facecolor': IVORY,
        'text.color': NAVY, 'axes.labelcolor': MUTED,
        'svg.hashsalt': 'pagecodec-end-to-end-report-v1',
    })
    fig = plt.figure(figsize=(17.6, 14.8))
    fig.text(.045, .964, 'PAGECODEC LAB  /  COMPLETE APPLICATION MEASUREMENTS',
             size=11.5, color=COBALT, weight='bold', va='top')
    fig.text(.045, .929, 'From source text to an exact report',
             size=28, weight='bold', va='top')
    fig.text(.045, .887, 'Parsing, construction, query evaluation and answer writing included.',
             size=14, color=MUTED, va='top')
    fig.text(.045, .862, 'Offline batch: every request is known before ingestion. Three real sources; generated, unique query plans.',
             size=12, color=MUTED, va='top')

    handles = [Line2D([], [], color=COLORS[m], marker=MARKERS[m], linestyle=STYLES[m],
                      linewidth=2.2, markersize=7, label=LABELS[m]) for m in METHODS]
    fig.legend(handles=handles, ncol=3, frameon=False, loc='upper left',
               bbox_to_anchor=(.040, .836), fontsize=11.3,
               columnspacing=2.4, handlelength=3, labelspacing=.9)

    fig.text(.115, .760, 'TOTAL ELAPSED TIME', size=13, weight='bold')
    fig.text(.565, .760, 'WHOLE-PROCESS PEAK MEMORY', size=13, weight='bold')
    fig.text(.115, .741, 'Milliseconds · logarithmic scale · lower is better', size=10.5, color=MUTED)
    fig.text(.565, .741, 'Peak RSS in MiB · linear scale from zero · lower is better', size=10.5, color=MUTED)

    top, row_height, gap = .697, .145, .051
    for row, source in enumerate(SOURCES):
        bottom = top - row_height - row*(row_height+gap)
        case = cases[source, COUNTS[-1]]
        label_y = bottom+row_height+.020
        fig.text(.045, label_y, SOURCE_LABELS[source], fontsize=15, weight='bold')
        fig.text(.955, label_y,
                 f"{case['source']['bytes']/2**20:,.2f} MiB source text  ·  {case['rows']:,} evaluation rows",
                 fontsize=10.5, color=MUTED, ha='right')
        for col, field in enumerate(('external_elapsed_ms', 'peak_rss_bytes')):
            axis = fig.add_axes([.115 if col == 0 else .565, bottom, .375, row_height])
            axis_style(axis, show_x=row == len(SOURCES)-1)
            scale = 1 if col == 0 else 2**20
            minima, maxima = [], []
            for method in METHODS:
                triples = [stats[source, count, method, field] for count in COUNTS]
                low, middle, high = ([v[i]/scale for v in triples] for i in range(3))
                minima.extend(low); maxima.extend(high)
                axis.fill_between(COUNTS, low, high, color=COLORS[method], alpha=.065, linewidth=0)
                axis.errorbar(COUNTS, middle,
                              yerr=([m-l for m,l in zip(middle, low)], [h-m for h,m in zip(high, middle)]),
                              fmt=MARKERS[method], linestyle=STYLES[method], color=COLORS[method],
                              linewidth=1.9 if method != 'stream_sweep' else 2.4,
                              markersize=5.5, markeredgecolor=IVORY, markeredgewidth=.65,
                              elinewidth=.9, capsize=2.3, capthick=.8, zorder=5 if method=='stream_sweep' else 3)
            if col == 0:
                axis.set_yscale('log')
                axis.set_ylim(min(minima)/1.45, max(maxima)*1.35)
                axis.yaxis.set_major_locator(LogLocator(base=10, subs=(1, 2, 5), numticks=12))
                axis.yaxis.set_major_formatter(FuncFormatter(lambda v, _: compact(v)))
                axis.yaxis.set_minor_locator(NullLocator())
            else:
                axis.set_ylim(0, max(maxima)*1.14)
                axis.yaxis.set_major_locator(MaxNLocator(nbins=5, min_n_ticks=4))
                axis.yaxis.set_major_formatter(FuncFormatter(lambda v, _: compact(v)))

    machine = report['frozen_config'].get('machine', '')
    chip = next((line.partition(':')[2].strip() for line in machine.splitlines()
                 if line.startswith('machdep.cpu.brand_string:')), report['frozen_config']['platform'])
    fig.text(.045, .109, 'Markers = nine-trial medians. Whiskers and faint envelopes = observed min–max, not confidence intervals.',
             size=10.8, color=MUTED)
    fig.text(.045, .088, 'Elapsed: parent clock around a fresh timed process, through output and exit. RSS: child process lifetime maximum.',
             size=10.8, color=MUTED)
    fig.text(.045, .067, 'Source preparation, calibration and oracle checks excluded. OS file cache not flushed. Streaming sweep retains no data columns.',
             size=10.4, color=MUTED)
    fig.text(.045, .041, f'{chip}  ·  one desktop session  ·  540 verified processes  ·  no application deployment or cold-disk claim',
             size=10.5, color=NAVY)
    try:
        source_label = directory.relative_to(ROOT).as_posix()
    except ValueError:
        source_label = directory.name
    fig.text(.045, .020, f'SOURCE  {source_label}/all-results.json', size=9.5, color=MUTED)
    fig.text(.955, .020, f'JSON SHA-256: {source_hash[:20]}…', size=9.5, color=MUTED, ha='right')

    description = ('Complete offline-report application benchmark. Five methods on three independent real sources; '
                   'four unique-query counts and nine fresh-process trials per method, 540 processes. '
                   'Times include query-plan reading, source parsing, construction, operations, output and process exit. '
                   'Memory is measured process peak RSS, not representation size. '
                   'Medians and observed min–max; no confidence interval. OS file cache not flushed; preparation excluded. '
                   f'Source JSON SHA-256: {source_hash}')
    fig.canvas.draw()
    renderer = fig.canvas.get_renderer()
    for label in fig.texts:
        box = label.get_window_extent(renderer)
        if box.x0 < 0 or box.x1 > fig.bbox.width or box.y0 < 0 or box.y1 > fig.bbox.height:
            raise ValueError(f'Text extends beyond canvas: {label.get_text()}')
    output.mkdir(parents=True, exist_ok=True)
    stem = output/'end-to-end-results'
    fig.savefig(stem.with_suffix('.png'), dpi=190,
                metadata={'Title': 'From source text to an exact report', 'Description': description})
    svg = stem.with_suffix('.svg')
    fig.savefig(svg, metadata={'Title': 'From source text to an exact report', 'Description': description, 'Date': None})
    svg.write_text('\n'.join(line.rstrip() for line in svg.read_text().splitlines())+'\n')
    plt.close(fig)
    print(f'Validated 540 measurements; wrote {stem.with_suffix(".png")} and {svg}.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=DEFAULT)
    parser.add_argument('--output-dir', type=Path, default=ROOT/'assets')
    args = parser.parse_args()
    render(args.results.resolve(), args.output_dir.resolve())
