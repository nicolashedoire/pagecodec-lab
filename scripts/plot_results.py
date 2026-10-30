#!/usr/bin/env python3
"""Render the published figures directly from measured benchmark JSON.

From the repository root:
    uv run --with-requirements requirements-figures.txt python scripts/plot_results.py

No benchmark or corpus download is performed. Default paths are relative to the
repository, regardless of the directory from which this script is invoked.
"""
import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.ticker import FixedLocator, FuncFormatter, NullLocator


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_RESULTS = ROOT / "results/household-2026-10-30/all-results.json"
METHODS = ("raw", "apple_lz4", "apple_lz4_raw", "apple_lzfse", "custom_rc")
LABELS = ("Raw copy", "Apple LZ4", "Apple LZ4_RAW", "Apple LZFSE", "Custom V2")
IVORY = "#FAF8F2"
NAVY = "#142C43"
COBALT = "#335DCB"
TEAL = "#008A80"
MUTED = "#5D6F7D"
GRID = "#DEDFF0"
COLORS = ("#8996A2", COBALT, "#708CCE", NAVY, TEAL)
PANELS = (
    ("Stored size", "Percent of input · linear scale", "storage"),
    ("Encode time", "Median milliseconds · log scale", "encode"),
    ("Decode time", "Median milliseconds · log scale", "decode"),
)


def load_results(path):
    report = json.loads(path.read_text())
    corpus = {}
    for dataset in report["corpus"]:
        rows = {row["method"]: row for row in dataset["results"]}
        if len(dataset["results"]) != len(METHODS) or set(rows) != set(METHODS):
            raise ValueError("Every dataset must contain all five distinct methods")
        if (dataset["repetitions"] != report["frozen_config"]["repetitions"]
                or dataset["page_size"] != report["frozen_config"]["page_bytes"]):
            raise ValueError("Measurement protocol differs from frozen configuration")
        if dataset["dataset"]["source_corpus"] != "uci-household-power-235":
            raise ValueError("These figures describe the UCI household-power corpus")
        if not all(row["verified"] for row in rows.values()):
            raise ValueError("Refusing to plot an unverified measurement")
        if dataset["input_bytes"] != dataset["dataset"]["size"]:
            raise ValueError("Input size differs from corpus metadata")
        for row in rows.values():
            if row["total_storage_bytes"] != row["payload_bytes"] + row["metadata_bytes"]:
                raise ValueError("Inconsistent storage accounting")
            for phase in ("encode", "decode"):
                samples = row[f"{phase}_samples_ms"]
                if len(samples) != dataset["repetitions"]:
                    raise ValueError("Unexpected number of timing samples")
                if not all(math.isfinite(value) and value > 0 for value in samples):
                    raise ValueError("Logarithmic timing plots require positive finite values")
                if not math.isclose(statistics.median(samples), row[f"{phase}_ms"], abs_tol=1e-6):
                    raise ValueError("Reported median differs from actual timing samples")
        corpus[dataset["dataset"]["name"]] = dataset
    return report, corpus


def style_axis(axis, show_methods):
    axis.set_facecolor(IVORY)
    axis.set_ylim(4.65, -0.70)
    axis.set_yticks(range(5), LABELS if show_methods else [""] * 5)
    axis.tick_params(axis="y", length=0, pad=14, labelsize=14)
    axis.tick_params(axis="x", length=0, pad=9, labelsize=12, colors=MUTED)
    for spine in axis.spines.values():
        spine.set_visible(False)
    axis.grid(axis="x", color=GRID, linewidth=0.8, zorder=0)
    axis.set_axisbelow(True)
    axis.axhspan(3.59, 4.41, facecolor=TEAL, alpha=0.06, zorder=0)
    if show_methods:
        for label, color in zip(axis.get_yticklabels(), COLORS):
            label.set_color(color)
        axis.get_yticklabels()[-1].set_fontweight("bold")


def timing_limits(all_corpus, phase):
    samples = [value for dataset in all_corpus.values() for row in dataset["results"]
               for value in row[f"{phase}_samples_ms"]]
    low_power = math.floor(math.log10(min(samples)))
    high_power = math.ceil(math.log10(max(samples)))
    return min(samples) / 1.45, 10 ** high_power * 1.55, [
        10 ** power for power in range(low_power, high_power + 1)
        if 10 ** power >= min(samples) / 1.45
    ]


def make_figure(report, all_corpus, specifications, output, text_mode, source_hash):
    plt.rcParams.update({
        "font.family": "DejaVu Sans", "font.size": 14,
        "text.color": NAVY, "axes.labelcolor": NAVY,
        "figure.facecolor": IVORY, "savefig.facecolor": IVORY,
        "svg.hashsalt": "pagecodec-lab-measured-results-v1",
    })
    fig = plt.figure(figsize=(14.8, 10.6))
    fig.text(0.045, 0.957, "PAGECODEC LAB / MEASURED RESULTS", color=COBALT,
             size=12, weight="bold", va="top")
    title = "Original text, two partitions" if text_mode else "Numerical data, two memory layouts"
    fig.text(0.045, 0.917, title, size=27, weight="bold", va="top")
    description = ("UCI household power · original bytes · two parts of ONE corpus"
                   if text_mode else
                   "UCI household power · exact fixed-point int32 · two views of ONE corpus")
    fig.text(0.045, 0.866, description, size=14.5, color=MUTED, va="top")
    config = report["frozen_config"]
    machine = next((line.split(":", 1)[1].strip() for line in config["machine"].splitlines()
                    if line.startswith("machdep.cpu.brand_string:")), "Machine recorded in JSON")
    protocol = (f"{machine}  ·  {config['repetitions']} randomized paired trials  ·  "
                f"{config['page_bytes'] // 1024} KiB pages  ·  lower is better")
    fig.text(0.045, 0.832, protocol, size=12.5, color=MUTED, va="top")

    starts = (0.225, 0.485, 0.745)
    width = 0.225
    for left, (heading, caption, _) in zip(starts, PANELS):
        fig.text(left, 0.781, heading, size=18, weight="bold")
        fig.text(left, 0.756, caption, size=11.5, color=MUTED)

    for row_index, (name, row_title) in enumerate(specifications):
        dataset = all_corpus[name]
        results = {row["method"]: row for row in dataset["results"]}
        bottom = 0.455 if row_index == 0 else 0.130
        row_heading = 0.713 if row_index == 0 else 0.393
        fig.text(0.045, row_heading,
                 f"{'A' if row_index == 0 else 'B'}  {row_title}", size=17, weight="bold")
        fig.text(0.97, row_heading, f"{dataset['input_bytes'] / 2**20:.2f} MiB input",
                 size=13, color=MUTED, ha="right")
        fig.add_artist(Line2D([0.045, 0.97], [row_heading - 0.016] * 2,
                             transform=fig.transFigure, color=GRID, linewidth=1))
        for column, (left, (_, _, metric)) in enumerate(zip(starts, PANELS)):
            axis = fig.add_axes([left, bottom, width, 0.220])
            style_axis(axis, column == 0)
            if metric == "storage":
                values = [100 * results[method]["total_storage_bytes"] / dataset["input_bytes"]
                          for method in METHODS]
                axis.barh(range(5), values, height=0.48, color=COLORS, zorder=2)
                axis.set_xlim(0, 126)
                axis.set_xticks([0, 50, 100], ["0%", "50%", "100%"])
                for y, (value, color) in enumerate(zip(values, COLORS)):
                    axis.annotate(f"{value:.1f}%", (value, y), xytext=(6, 0),
                                  textcoords="offset points", va="center", fontsize=13,
                                  color=color, weight="bold" if y == 4 else "normal")
            else:
                low, high, ticks = timing_limits(all_corpus, metric)
                axis.set_xscale("log")
                axis.set_xlim(low, high)
                axis.xaxis.set_major_locator(FixedLocator(ticks))
                axis.xaxis.set_major_formatter(FuncFormatter(lambda value, _: f"{value:g}"))
                axis.xaxis.set_minor_locator(NullLocator())
                for y, (method, color) in enumerate(zip(METHODS, COLORS)):
                    row = results[method]
                    median = row[f"{metric}_ms"]
                    samples = row[f"{metric}_samples_ms"]
                    axis.hlines(y, min(samples), max(samples), color=color,
                                alpha=0.4, linewidth=3, zorder=2)
                    axis.scatter([median], [y], s=77 if y == 4 else 65, color=color,
                                 edgecolor=IVORY, linewidth=1, zorder=3)
                    axis.annotate(f"{median:.2f}", (median, y), xytext=(6, 8),
                                  textcoords="offset points", va="bottom", fontsize=13,
                                  color=color, weight="bold" if y == 4 else "normal")

    fig.text(0.045, 0.058,
             "Stored size includes page metadata. Dots = medians; lines = observed min–max, not confidence intervals.",
             size=11.5, color=MUTED)
    fig.text(0.045, 0.029,
             "Single-thread warm buffers; raw = copying. Apple Compression APIs, not the macOS memory compressor.",
             size=11.5, color=MUTED)
    fig.text(0.97, 0.004, f"Source JSON SHA-256: {source_hash[:16]}…", size=8.5,
             color=MUTED, ha="right", va="bottom")
    output.parent.mkdir(parents=True, exist_ok=True)
    metadata = {"Title": title, "Description": description + ". " + protocol}
    fig.savefig(output.with_suffix(".png"), dpi=180, metadata=metadata)
    svg_path = output.with_suffix(".svg")
    fig.savefig(svg_path, metadata={**metadata, "Date": None})
    # Matplotlib emits spaces at line endings in path data; keep generated
    # assets clean under the repository's normal whitespace checks.
    svg_path.write_text("\n".join(line.rstrip() for line in svg_path.read_text().splitlines()) + "\n")
    plt.close(fig)
    print(f"Wrote {output.with_suffix('.png')} and {output.with_suffix('.svg')}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, default=DEFAULT_RESULTS)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "assets")
    args = parser.parse_args()
    report, corpus = load_results(args.results)
    source_hash = hashlib.sha256(args.results.read_bytes()).hexdigest()
    make_figure(report, corpus, [
        ("household-row-i32.bin", "Row-major integers"),
        ("household-column-i32.bin", "Column-major integers"),
    ], args.output_dir / "numeric-results", False, source_hash)
    make_figure(report, corpus, [
        ("household-text-part-01.txt", "Original text · part 1"),
        ("household-text-part-02.txt", "Original text · part 2"),
    ], args.output_dir / "text-results", True, source_hash)


if __name__ == "__main__":
    main()
