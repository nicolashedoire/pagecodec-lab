#!/usr/bin/env python3
"""Draw audited native-compaction figures from measurements; never benchmark.

Requires requirements-figures.txt. Pilot input produces pilot-labelled filenames
and figures. The shared summarizer validates pairing and integrity first.
"""
import argparse
import math
from pathlib import Path

import matplotlib

matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.colors import LinearSegmentedColormap, Normalize, TwoSlopeNorm
from matplotlib.patches import Rectangle
import numpy as np

from summarize_compaction import DEFAULT, POLICIES, ROOT, SOURCES, build_summary

IVORY, NAVY, TEAL, COBALT = '#FAF8F2', '#142C43', '#008A80', '#335DCB'
MUTED, GRID, RUST = '#637787', '#DBE1E5', '#B95542'
SOURCE_LABELS = {'household': 'Household power', 'covertype': 'Covertype',
                 'rea_source': 'Rea source code', 'random_control': 'Random control'}
ROWS = (('front', .5), ('tail', .5), ('random', .25), ('random', .5),
        ('random', 1.), ('mixed', .25), ('mixed', .5), ('mixed', 1.))
COST_ROWS = (('none', .5), ('subpage', .5), *ROWS)
NAMES = {'none': 'No deletions', 'subpage': 'Subpage holes', 'front': 'Front holes',
         'tail': 'Tail holes', 'random': 'Random holes', 'mixed': 'Mixed layout'}
SHORT = {'fifo': 'FIFO', 'benefit': 'BEN', 'efficiency': 'EFF'}


def label(row):
    layout, fraction = row
    if layout in ('none', 'subpage'):
        return NAMES[layout]
    return f'{NAMES[layout]} · {fraction:.0%}' + (' †' if fraction == 1 else '')


def style():
    plt.rcParams.update({
        'font.family': 'DejaVu Sans', 'font.size': 11,
        'figure.facecolor': IVORY, 'savefig.facecolor': IVORY,
        'text.color': NAVY, 'axes.labelcolor': MUTED,
        'svg.hashsalt': 'pagecodec-native-compaction-v1',
    })


def setup_grid(axis, columns, rows, *, row_labels=True):
    axis.set_facecolor(IVORY)
    axis.set_xticks(range(len(columns)), labels=columns)
    axis.set_yticks(range(len(rows)), labels=[label(row) for row in rows] if row_labels else ['']*len(rows))
    axis.tick_params(axis='both', length=0, labelsize=10, pad=9, colors=MUTED)
    axis.tick_params(axis='x', top=True, labeltop=True, bottom=False, labelbottom=False)
    axis.set_xticks(np.arange(-.5, len(columns), 1), minor=True)
    axis.set_yticks(np.arange(-.5, len(rows), 1), minor=True)
    axis.grid(which='minor', color=IVORY, linewidth=3)
    axis.tick_params(which='minor', bottom=False, left=False)
    for spine in axis.spines.values():
        spine.set_visible(False)
    for i, (_, fraction) in enumerate(rows):
        if fraction == 1:
            axis.add_patch(Rectangle((-.5, i-.5), len(columns), 1, fill=False,
                                     edgecolor='#9D8B64', linewidth=1.2, zorder=5))


def annotate(axis, values, *, norm, cmap, secondary=None, fontsize=12):
    for y, row in enumerate(values):
        for x, value in enumerate(row):
            if math.isnan(value):
                axis.text(x, y, '—', ha='center', va='center', color=MUTED, size=fontsize)
                continue
            rgb = cmap(norm(value))[:3]
            luminance = .2126*rgb[0] + .7152*rgb[1] + .0722*rgb[2]
            color = IVORY if luminance < .53 else NAVY
            if secondary is not None:
                text = f'{value:+.0f}%' if abs(value) >= 10 else f'{value:+.1f}%'
                axis.text(x, y-.12, text, ha='center', va='center', color=color,
                          size=fontsize, weight='bold')
                axis.text(x, y+.22, secondary[y][x], ha='center', va='center',
                          color=color, size=8.5)
            else:
                text = f'{value:.0f}%' if abs(value) >= 10 else f'{value:.1f}%'
                axis.text(x, y, text, ha='center', va='center', color=color, size=fontsize)


def source_footer(fig, summary, location, y=.025):
    path = location / 'all-results.json' if location.is_dir() else location
    try:
        source = path.relative_to(ROOT).as_posix()
    except ValueError:
        source = path.name
    fig.text(.045, y, f'SOURCE  {source}', size=9, color=MUTED)
    fig.text(.955, y, f'JSON SHA-256: {summary["source_sha256"][:20]}…',
             size=9, color=MUTED, ha='right')


def pilot_badge(fig, summary):
    if summary['pilot']:
        fig.text(.955, .964, 'PILOT ONLY · ONE PAIRED ROUND', size=11,
                 color=RUST, weight='bold', ha='right', va='top',
                 bbox={'facecolor': '#F7E3DA', 'edgecolor': 'none', 'pad': 7})


def save(fig, stem, title, description):
    description += f' Rendered with Matplotlib {matplotlib.__version__}.'
    fig.canvas.draw()
    renderer = fig.canvas.get_renderer()
    for text in fig.texts:
        box = text.get_window_extent(renderer)
        if box.x0 < 0 or box.x1 > fig.bbox.width or box.y0 < 0 or box.y1 > fig.bbox.height:
            raise ValueError(f'Text extends beyond canvas: {text.get_text()}')
    stem.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(stem.with_suffix('.png'), dpi=190,
                metadata={'Title': title, 'Description': description})
    svg = stem.with_suffix('.svg')
    fig.savefig(svg, metadata={'Title': title, 'Description': description, 'Date': None})
    svg.write_text('\n'.join(line.rstrip() for line in svg.read_text().splitlines()) + '\n')
    plt.close(fig)


def render_comparison(summary, index, location, output):
    values, captions = {}, {}
    for policy in ('benefit', 'efficiency'):
        values[policy], captions[policy] = [], []
        for layout, fraction in ROWS:
            row = [index[source, layout, fraction, policy] for source in SOURCES]
            values[policy].append([r['versus_fifo']['elapsed_ms']['paired_time_saved_pct']['median'] for r in row])
            captions[policy].append([f'{r["versus_fifo"]["elapsed_ms"]["faster_trials"]}/{r["trials"]} faster' for r in row])
    all_values = [v for matrix in values.values() for row in matrix for v in row]
    low = min(-5., math.floor(min(all_values)/5)*5)
    high = max(5., math.ceil(max(all_values)/5)*5)
    norm = TwoSlopeNorm(vmin=low, vcenter=0, vmax=high)
    cmap = LinearSegmentedColormap.from_list('compaction-savings', [RUST, '#F1EDE1', TEAL])
    fig = plt.figure(figsize=(17.6, 11.3))
    fig.text(.045, .964, 'PAGECODEC LAB  /  NATIVE MEMORY COMPACTION', size=11.5,
             color=COBALT, weight='bold', va='top')
    pilot_badge(fig, summary)
    fig.text(.045, .919, 'Which segments should be compacted first?',
             size=27, weight='bold', va='top')
    fig.text(.045, .873, 'Time saved versus eligible FIFO at a shared reclaim target',
             size=16, color=MUTED, va='top')
    fig.text(.045, .839, 'Real compressed payloads · synthetic deletion layouts · native decision, copies, handle updates and unmapping',
             size=11.5, color=MUTED, va='top')
    for col, policy in enumerate(('benefit', 'efficiency')):
        left = .185 if col == 0 else .605
        title = 'Benefit first' if col == 0 else 'Reclaim / movement efficiency'
        fig.text(left, .787, title, size=16, weight='bold')
        axis = fig.add_axes([left, .279, .35, .455])
        matrix = np.array(values[policy])
        axis.imshow(matrix, cmap=cmap, norm=norm, aspect='auto', interpolation='nearest')
        setup_grid(axis, [SOURCE_LABELS[s].replace(' ', '\n', 1) for s in SOURCES],
                   ROWS, row_labels=col == 0)
        annotate(axis, matrix, norm=norm, cmap=cmap, secondary=captions[policy])
    cax = fig.add_axes([.35, .216, .45, .018])
    colorbar = fig.colorbar(plt.cm.ScalarMappable(norm=norm, cmap=cmap), cax=cax,
                           orientation='horizontal')
    colorbar.set_label('← more time than FIFO                 time saved (%)                 less time than FIFO →',
                       size=10.5, labelpad=9)
    colorbar.outline.set_visible(False)
    colorbar.ax.tick_params(length=0, colors=MUTED, labelsize=9)
    n = summary['validation']['paired_trials_per_scenario']
    fig.text(.045, .140, f'Cells: median of {n} paired trial-by-trial time savings. Small text: trials faster than matching FIFO. Positive values save time.',
             size=10.5, color=MUTED)
    fig.text(.045, .117, '† Full-reclaim controls: all policies release the same bytes and move the same bytes in each paired round.',
             size=10.5, color=MUTED)
    fig.text(.045, .094, 'Percentages are relative to the native phase, not whole-program speedups. Quota overshoot is retained in the summary.',
             size=10.5, color=MUTED)
    fig.text(.045, .071, 'FIFO uses a page eligibility guard; this is not a comparison with the full Apple policy or a macOS kernel modification.',
             size=10.5, color=NAVY)
    source_footer(fig, summary, location)
    prefix = 'compaction-pilot' if summary['pilot'] else 'compaction'
    description = ('PILOT ONLY. ' if summary['pilot'] else '') + (
        f'Native user-space compressed-segment compaction; {n} paired trials per policy/scenario. '
        'Cell values are medians of 100*(1-alternative/FIFO), including selection overhead. '
        'Real compressed corpus payloads, replicated as disclosed in the manifest; synthetic deletion layouts. '
        'Full-reclaim rows are negative controls with identical copied/released bytes. '
        f'No full Apple-policy or kernel improvement claim. Source SHA-256 {summary["source_sha256"]}.')
    save(fig, output / f'{prefix}-results', 'Choosing compressed segments for compaction', description)


def render_costs(summary, index, location, output):
    columns = [(source, policy) for source in SOURCES for policy in POLICIES]
    decisions, recoveries = [], []
    for layout, fraction in COST_ROWS:
        rows = [index[source, layout, fraction, policy] for source, policy in columns]
        decisions.append([r['metrics']['decision_pct_of_elapsed']['median']
                          if 'decision_pct_of_elapsed' in r['metrics'] else math.nan for r in rows])
        recoveries.append([100*r['metrics']['rss_recovery_fraction_of_release']['median']
                           if 'rss_recovery_fraction_of_release' in r['metrics'] else math.nan for r in rows])
    fig = plt.figure(figsize=(17.6, 14.5))
    fig.text(.045, .969, 'PAGECODEC LAB  /  COST AND MEMORY CHECKS', size=11.5,
             color=COBALT, weight='bold', va='top')
    pilot_badge(fig, summary)
    fig.text(.045, .935, 'Selection has a cost. Released pages need a check.',
             size=25, weight='bold', va='top')
    trials = summary['comparisons'][0]['trials']
    fig.text(.045, .900, f'Median ratios across {trials} paired runs. Missing detailed measurements are not zero; no-deletion and subpage cases have zero quota.',
             size=12, color=MUTED, va='top')
    specs = [
        ('SEPARATELY MEASURED DECISION SHARE OF NATIVE ELAPSED TIME', decisions, .542, .277,
         LinearSegmentedColormap.from_list('decision-cost', ['#F3EEE0', COBALT]), Normalize(0, 100),
         'Decision time / native elapsed (%)'),
        ('RESIDENT DROP RELATIVE TO UNMAPPED BYTES', recoveries, .169, .277,
         LinearSegmentedColormap.from_list('rss-check', ['#F3EEE0', TEAL]),
         Normalize(min(0, min(v for row in recoveries for v in row if math.isfinite(v))),
                   max(100, max(v for row in recoveries for v in row if math.isfinite(v)))),
         'Measured resident drop / unmapped bytes (%)'),
    ]
    for heading, values, bottom, height, cmap, norm, colorbar_title in specs:
        fig.text(.185, bottom+height+.045, heading, size=13, weight='bold')
        axis = fig.add_axes([.185, bottom, .70, height])
        matrix = np.array(values)
        cmap.set_bad('#EAE9E3')
        axis.imshow(np.ma.masked_invalid(matrix), cmap=cmap, norm=norm, aspect='auto', interpolation='nearest')
        setup_grid(axis, [SHORT[p] for _, p in columns], COST_ROWS)
        axis.tick_params(axis='x', labelsize=8.5, pad=6)
        axis.tick_params(axis='y', labelsize=10)
        for group, source in enumerate(SOURCES):
            center = .185 + .70 * (group+.5)/len(SOURCES)
            fig.text(center, bottom+height+.032, SOURCE_LABELS[source], size=11,
                     ha='center', color=NAVY, weight='bold')
            if group:
                axis.axvline(group*3-.5, color=IVORY, linewidth=7)
        annotate(axis, matrix, norm=norm, cmap=cmap, fontsize=8.8)
        cax = fig.add_axes([.908, bottom, .012, height])
        colorbar = fig.colorbar(plt.cm.ScalarMappable(norm=norm, cmap=cmap), cax=cax)
        colorbar.outline.set_visible(False)
        colorbar.ax.tick_params(length=0, labelsize=8, colors=MUTED)
        colorbar.set_label(colorbar_title, size=9, labelpad=8)
    fig.text(.045, .113, 'FIFO = eligible queue · BEN = largest reclaim · EFF = reclaim / (predicted moved bytes + one OS page). † = full reclaim.',
             size=10.5, color=MUTED)
    fig.text(.045, .090, 'RSS cells are medians, not equality for every run. All outliers are retained in the report; RSS corroborates rather than defines page release.',
             size=10.3, color=MUTED)
    fig.text(.045, .068, '— = untimed decision component (top) or no bytes unmapped (bottom). Setup and integrity checks are outside the native timing.',
             size=10.3, color=MUTED)
    source_footer(fig, summary, location)
    prefix = 'compaction-pilot' if summary['pilot'] else 'compaction'
    description = ('PILOT ONLY. ' if summary['pilot'] else '') + (
        'Selection overhead and resident-memory corroboration for every native compaction scenario. '
        'Decision percentage and resident recovery percentage are medians of per-trial ratios. '
        'No-release cells have no defined resident-recovery ratio. No full-program or macOS-kernel gain claim. '
        f'Source SHA-256 {summary["source_sha256"]}.')
    save(fig, output / f'{prefix}-costs', 'Compaction decision costs and resident-memory corroboration', description)


def render(location, output):
    summary = build_summary(location)
    index = {(r['case'], r['layout'], r['reclaim_fraction'], r['policy']): r
             for r in summary['comparisons']}
    style()
    render_comparison(summary, index, location, output)
    render_costs(summary, index, location, output)
    label_text = 'PILOT ONLY' if summary['pilot'] else 'FROZEN FINAL RUN'
    print(f'{label_text}: validated {summary["validation"]["measurement_count"]} samples; wrote two SVG/PNG figures in {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=DEFAULT)
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'assets')
    args = parser.parse_args()
    render(args.results.resolve(), args.output_dir.resolve())
