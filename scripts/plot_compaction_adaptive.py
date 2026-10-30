#!/usr/bin/env python3
"""Plot validated adaptive-compaction measurements without running benchmarks.

Produces paired native-phase comparisons with FIFO and the prior efficiency
policy. Requires requirements-figures.txt. Pilot inputs cannot produce final
filenames or an experimental verdict.
"""
import argparse
import math
from pathlib import Path

import matplotlib

matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.colors import LinearSegmentedColormap, TwoSlopeNorm
from matplotlib.patches import Rectangle
import numpy as np

from summarize_compaction_adaptive import ROOT, SOURCES, POLICIES, build_summary

DEFAULT = ROOT / 'results/compaction-adaptive-2026-10-30'
PUBLIC_SOURCE = 'results/compaction-adaptive-2026-10-30/all-results.json'
IVORY, NAVY, TEAL, COBALT = '#FAF8F2', '#142C43', '#008A80', '#335DCB'
MUTED, RUST, CONTROL = '#637787', '#B95542', '#9D8B64'
ROWS = (
    ('front', .5), ('tail', .5),
    ('random', .25), ('random', .5), ('random', 1.),
    ('mixed', .25), ('mixed', .5), ('mixed', 1.),
    ('scrambled', .25), ('scrambled', .5), ('scrambled', 1.),
    ('rare_tail', .25), ('rare_tail', .5), ('rare_tail', 1.),
)
SOURCE_LABELS = {
    'household_row': 'Household\nrows', 'bike': 'Bike\nrecords',
    'cmu_pronunciations': 'CMU\npronunciations', 'random_control': 'Random\ncontrol',
}
LAYOUT_LABELS = {
    'front': 'Front holes', 'tail': 'Tail holes', 'random': 'Random holes',
    'mixed': 'Mixed layout', 'scrambled': 'Scrambled', 'rare_tail': 'Rare tails',
}
POLICY_LABELS = {'tail_first': 'Tail first', 'adaptive': 'Adaptive rule'}
BASELINE_LABELS = {'fifo': 'eligible FIFO', 'efficiency': 'the original efficiency score'}


def row_label(row):
    layout, fraction = row
    return f'{LAYOUT_LABELS[layout]} · {fraction:.0%}' + (' †' if fraction == 1 else '')


def style():
    plt.rcParams.update({
        'font.family': 'DejaVu Sans', 'font.size': 11,
        'figure.facecolor': IVORY, 'savefig.facecolor': IVORY,
        'text.color': NAVY, 'axes.labelcolor': MUTED,
        'svg.hashsalt': 'pagecodec-adaptive-compaction-v1',
    })


def extract(summary, baseline):
    if not {'tail_first', 'adaptive', baseline}.issubset(POLICIES):
        raise ValueError('Unexpected policy set in adaptive summary module')
    index = {(r['case'], r['layout'], float(r['reclaim_fraction']), r['policy']): r
             for r in summary['comparisons']}
    matrices, captions = {}, {}
    for policy in ('tail_first', 'adaptive'):
        matrix, secondary = [], []
        for layout, fraction in ROWS:
            entries = [index[source, layout, fraction, policy] for source in SOURCES]
            comparisons = [r['versus'][baseline]['elapsed_ms'] for r in entries]
            values = [c['paired_time_saved_pct']['median'] for c in comparisons]
            if not all(isinstance(v, (int, float)) and math.isfinite(v) for v in values):
                raise ValueError(f'Invalid paired percentages for {layout}/{fraction}/{policy}')
            matrix.append(values)
            secondary.append([f'{c["faster_trials"]}/{r["trials"]} faster'
                              for c, r in zip(comparisons, entries)])
        matrices[policy], captions[policy] = np.array(matrix), secondary
    return matrices, captions


def grid_style(axis, *, show_rows):
    axis.set_facecolor(IVORY)
    axis.set_xticks(range(len(SOURCES)), labels=[SOURCE_LABELS[s] for s in SOURCES])
    axis.set_yticks(range(len(ROWS)), labels=[row_label(r) for r in ROWS] if show_rows else ['']*len(ROWS))
    axis.tick_params(axis='both', length=0, labelsize=10.5, pad=9, colors=MUTED)
    axis.tick_params(axis='x', top=True, labeltop=True, bottom=False, labelbottom=False)
    axis.set_xticks(np.arange(-.5, len(SOURCES), 1), minor=True)
    axis.set_yticks(np.arange(-.5, len(ROWS), 1), minor=True)
    axis.grid(which='minor', color=IVORY, linewidth=2.8)
    axis.tick_params(which='minor', bottom=False, left=False)
    for spine in axis.spines.values():
        spine.set_visible(False)
    for y, (layout, fraction) in enumerate(ROWS):
        if fraction == 1:
            axis.add_patch(Rectangle((-.5, y-.5), len(SOURCES), 1, fill=False,
                                     edgecolor=CONTROL, linewidth=1.1, zorder=5))
        if layout == 'scrambled' and show_rows:
            axis.get_yticklabels()[y].set_color(COBALT)
            axis.get_yticklabels()[y].set_weight('bold')
    for boundary in (1.5, 4.5, 7.5, 10.5):
        axis.axhline(boundary, color=IVORY, linewidth=6, zorder=4)


def annotate(axis, matrix, secondary, norm, cmap):
    for y, row in enumerate(matrix):
        for x, value in enumerate(row):
            rgb = cmap(norm(value))[:3]
            luminance = .2126*rgb[0] + .7152*rgb[1] + .0722*rgb[2]
            color = IVORY if luminance < .53 else NAVY
            # Avoid displaying a tiny negative measurement as a positive gain.
            formatted = f'{value:+.0f}%' if abs(value) >= 10 else f'{value:+.1f}%'
            axis.text(x, y-.14, formatted, ha='center', va='center', color=color,
                      size=12, weight='bold')
            axis.text(x, y+.22, secondary[y][x], ha='center', va='center',
                      color=color, size=8.5)


def criterion_text(summary):
    if summary['pilot']:
        return 'Pilot checks execution and integrity only; no final success claim.', RUST
    criterion = summary['success_criterion']
    passed = criterion.get('passed')
    if not isinstance(passed, bool):
        raise ValueError('Final summary must contain a boolean success criterion verdict')
    count = len(criterion['passing_same_sources'])
    status = 'MET' if passed else 'NOT MET'
    return (f'Predeclared exploratory criterion: {status} · {count}/3 real sources meet both partial-target checks; full-target guard included.',
            TEAL if passed else RUST)


def source_label(summary, location):
    if not summary['pilot']:
        return PUBLIC_SOURCE
    path = location / 'all-results.json' if location.is_dir() else location
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.name


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


def render_comparison(summary, baseline, location, output):
    values, captions = extract(summary, baseline)
    flattened = np.concatenate([m.ravel() for m in values.values()])
    low = min(-5., math.floor(float(flattened.min())/5)*5)
    high = max(5., math.ceil(float(flattened.max())/5)*5)
    norm = TwoSlopeNorm(vmin=low, vcenter=0, vmax=high)
    cmap = LinearSegmentedColormap.from_list('adaptive-compaction-savings', [RUST, '#F1EDE1', TEAL])
    fig = plt.figure(figsize=(17.6, 16.2))
    fig.text(.045, .970, 'PAGECODEC LAB  /  ADAPTIVE MEMORY COMPACTION',
             size=11.5, color=COBALT, weight='bold', va='top')
    if summary['pilot']:
        fig.text(.955, .970, 'PILOT ONLY · ONE PAIRED ROUND', size=11,
                 color=RUST, weight='bold', ha='right', va='top',
                 bbox={'facecolor': '#F7E3DA', 'edgecolor': 'none', 'pad': 7})
    fig.text(.045, .940, 'Can a simpler decision reclaim memory faster?',
             size=27, weight='bold', va='top')
    fig.text(.045, .907, f'Time saved versus {BASELINE_LABELS[baseline]} at a shared reclaim target',
             size=15, color=MUTED, va='top')
    fig.text(.045, .882, 'Real compressed payloads · synthetic deletion layouts · native selection, data movement, handle updates and unmapping',
             size=11.2, color=MUTED, va='top')
    if baseline == 'fifo':
        text, color = criterion_text(summary)
    else:
        text, color = ('Reference score: reclaimable bytes / (predicted moved bytes + one OS page). Its scanning and sorting are timed.', MUTED)
    fig.text(.045, .858, text, size=10.8, color=color)
    for col, policy in enumerate(('tail_first', 'adaptive')):
        left = .185 if col == 0 else .605
        fig.text(left, .826, POLICY_LABELS[policy], size=16, weight='bold')
        detail = 'Zero-copy candidates first; no sorting' if policy == 'tail_first' else 'Own capacity scan; FIFO at the full target'
        fig.text(left, .808, detail, size=10.8, color=MUTED)
        axis = fig.add_axes([left, .252, .35, .515])
        axis.imshow(values[policy], cmap=cmap, norm=norm, aspect='auto', interpolation='nearest')
        grid_style(axis, show_rows=col == 0)
        annotate(axis, values[policy], captions[policy], norm, cmap)
    cax = fig.add_axes([.35, .214, .45, .014])
    bar = fig.colorbar(plt.cm.ScalarMappable(norm=norm, cmap=cmap), cax=cax, orientation='horizontal')
    bar.set_label('← more time than the reference             time saved (%)             less time than the reference →',
                  size=10.5, labelpad=9)
    bar.outline.set_visible(False)
    bar.ax.tick_params(length=0, colors=MUTED, labelsize=9)
    trials = summary['comparisons'][0]['trials']
    fig.text(.045, .151, f'Cells: median of {trials} paired trial-by-trial percentages. Small text: faster trials. Positive values save time; both panels share a scale.',
             size=10.2, color=MUTED)
    fig.text(.045, .130, '† Full-target controls: equal bytes moved and released per pair. Blue labels mark the predeclared scrambled checks against FIFO.',
             size=10.2, color=MUTED)
    fig.text(.045, .109, 'Setup, source compression and integrity checks are separate. Extra capacity and tail scans are included; this is not whole-program timing.',
             size=10.2, color=MUTED)
    fig.text(.045, .088, 'Front, tail, mixed and rare-tail layouts are deterministic; random and scrambled layouts vary by seed. These are not production traces.',
             size=10.2, color=MUTED)
    fig.text(.045, .067, 'Household reuses a source from V1; Bike and CMU are new to compaction. Corpus repetitions do not add independent data.',
             size=10.2, color=MUTED)
    fig.text(.045, .046, 'A controlled user-space mechanism experiment: no generalization claim, complete Apple-policy comparison or macOS kernel modification.',
             size=10.2, color=NAVY)
    fig.text(.045, .020, f'SOURCE  {source_label(summary, location)}', size=9, color=MUTED)
    fig.text(.955, .020, f'JSON SHA-256: {summary["source_sha256"][:20]}…', size=9, color=MUTED, ha='right')
    prefix = 'compaction-adaptive-pilot' if summary['pilot'] else 'compaction-adaptive'
    suffix = 'results' if baseline == 'fifo' else 'versus-efficiency'
    description = ('PILOT ONLY. ' if summary['pilot'] else '') + (
        f'Native user-space compressed-segment compaction; {trials} paired trials per policy and scenario. '
        f'Comparisons with {baseline}; percentages are medians of 100*(1-policy/reference), not ratios of medians. '
        'All 14 scenario/quota combinations and four corpora are shown, including regressions. '
        'Full-target controls require identical bytes moved and released. Selection scans are included. '
        'Synthetic deletion layouts, repeated source data, no production generalization or macOS-kernel claim. '
        f'Source {source_label(summary, location)}; SHA-256 {summary["source_sha256"]}.')
    save(fig, output/f'{prefix}-{suffix}', f'Adaptive compaction compared with {BASELINE_LABELS[baseline]}', description)


def render(location, output):
    summary = build_summary(location)
    style()
    for baseline in ('fifo', 'efficiency'):
        render_comparison(summary, baseline, location, output)
    status = 'PILOT ONLY' if summary['pilot'] else 'FROZEN FINAL RUN'
    print(f'{status}: wrote FIFO and prior-efficiency SVG/PNG comparisons to {output}; Matplotlib {matplotlib.__version__}.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, default=DEFAULT)
    parser.add_argument('--output-dir', type=Path, default=ROOT/'assets')
    args = parser.parse_args()
    render(args.results.resolve(), args.output_dir.resolve())
