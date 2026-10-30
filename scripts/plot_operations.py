#!/usr/bin/env python3
"""Render three operation figures from validated measurement CSVs and source JSON.

Run summarize_operations.py first. This script performs no benchmarks/downloads.
From the repository root:
  uv run --quiet --with-requirements requirements-figures.txt python scripts/plot_operations.py
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.colors import LinearSegmentedColormap, Normalize
from matplotlib.ticker import FuncFormatter, MaxNLocator, NullLocator
import numpy as np


ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / "results/operations-2026-10-30"
IVORY, NAVY, COBALT, TEAL = "#FAF8F2", "#142C43", "#335DCB", "#008A80"
MUTED, GRID, BASE = "#687987", "#DDDDE1", "#8E9DA8"
FAMILIES = {
    "query": {
        "title": "A  |  Exact masked queries",
        "subtitle": "Lossless integer columns · COUNT and masked SUM · no full output materialization",
        "baseline": "raw",
        "baseline_label": "raw NEON",
        "methods": ["raw", "for", "bitplane_eager", "bitplane_progressive", "bitplane_adaptive", "rle"],
        "labels": ["Raw", "FOR", "Eager bitplanes", "Progressive bitplanes", "Adaptive bitplanes", "Pair RLE"],
        "cases": ["household", "covertype", "bike"],
        "memory_note": "Working storage: owned representation capacities + explicit scratch; allocator overhead excluded. Main-run stores coexist.",
        "build_note": "Build: one measured construction per method/corpus; dots are observations, not uncertainty estimates.",
        "workloads": None,
    },
    "recipe": {
        "title": "B  |  Store values or recompute?",
        "subtitle": "Eight exact integer features · retained source arrays · four access patterns",
        "baseline": "materialized",
        "baseline_label": "materialized",
        "methods": ["materialized", "compressed_blocks", "recipe", "recipe_cache", "recipe_direct"],
        "labels": ["Materialized", "Compressed tiles", "Recipe: one tile", "Recipe: tile cache", "Recipe: direct"],
        "cases": ["household", "covertype", "bike"],
        "memory_note": "Working storage: source arrays + derived representation + indexes/objects + cache/scratch + common schedule.",
        "build_note": "Build: nine-trial median and min–max; 0* means below recorded resolution, not free setup. Logical caches reset before workloads.",
        "workloads": ["sequential_all", "repeated_hot_windows", "sparse_points", "changing_hotset"],
    },
    "strings": {
        "title": "C  |  Shared string storage",
        "subtitle": "Exact object reconstruction · uniform and hot-set access · view and copy contracts",
        "baseline": "concat",
        "baseline_label": "concatenation",
        "methods": ["concat", "intern", "bounded_overlap", "zlib_blocks"],
        "labels": ["Concatenation", "Interning", "Bounded overlap", "Zlib blocks"],
        "cases": ["cmu-headwords", "cmu-pronunciations"],
        "memory_note": "Working storage: store capacity + copy/decode workspace; common trace and object headers excluded.",
        "build_note": "Build: median and observed min–max of nine constructions. The two panels are views of one CMU source corpus.",
        "workloads": ["uniform.view", "uniform.copy", "hot.view", "hot.copy"],
    },
}
CASE_LABELS = {
    "household": "Household power", "covertype": "Covertype", "bike": "Bike sharing",
    "cmu-headwords": "CMU dictionary · headwords", "cmu-pronunciations": "CMU dictionary · pronunciations",
}
WORKLOAD_LABELS = {
    "sequential_all": "Full scan\nall features", "repeated_hot_windows": "Repeated\nhot windows",
    "sparse_points": "Random\npoint reads", "changing_hotset": "Changing\nhot set",
    "uniform.view": "Uniform\nview", "uniform.copy": "Uniform\ncopy",
    "hot.view": "Hot set\nview", "hot.copy": "Hot set\ncopy",
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_csv(path):
    with path.open(newline="") as stream:
        return list(csv.DictReader(stream))


def number(row, key):
    value = float(row[key])
    if not math.isfinite(value):
        raise ValueError(f"Non-finite {key}: {row}")
    return value


def load(directory):
    paths = {name: directory / name for name in ("all-results.json", "timings.csv", "memory.csv", "construction.csv", "timing-samples.csv")}
    for path in paths.values():
        if not path.is_file():
            raise FileNotFoundError(f"Missing {path}; finish measurements and run scripts/summarize_operations.py first")
    report = json.loads(paths["all-results.json"].read_text())
    timings, memory, builds, samples = (read_csv(paths[name]) for name in ("timings.csv", "memory.csv", "construction.csv", "timing-samples.csv"))
    case_map = {(case["family"], case["case"]["name"]): case for case in report["experiments"]}
    timing_map, memory_map, build_map = {}, {}, {}
    for row in timings:
        key = row["family"], row["corpus"], row["method"], row["workload"]
        if key in timing_map:
            raise ValueError(f"Duplicate timing cell: {key}")
        if not 0 < number(row, "minimum") <= number(row, "median") <= number(row, "maximum"):
            raise ValueError(f"Invalid observed timing range: {key}")
        timing_map[key] = row
    for row in memory:
        key = row["family"], row["corpus"], row["method"]
        if key in memory_map or number(row, "accounted_working_bytes") <= 0:
            raise ValueError(f"Invalid memory cell: {key}")
        memory_map[key] = row
    for row in builds:
        key = row["family"], row["corpus"], row["method"]
        if key in build_map or not 0 <= number(row, "minimum_ms") <= number(row, "median_ms") <= number(row, "maximum_ms"):
            raise ValueError(f"Invalid construction cell: {key}")
        build_map[key] = row
    sample_map = {}
    for row in samples:
        key = row["family"], row["corpus"], row["method"], row["workload"]
        sample_map.setdefault(key, []).append(number(row, "value"))
    for key, row in timing_map.items():
        values = sorted(sample_map.get(key, []))
        if len(values) != 9 or not math.isclose(values[4], number(row, "median"), rel_tol=1e-8):
            raise ValueError(f"Expected nine consistent trial samples: {key}")
        if not math.isclose(values[0], number(row, "minimum"), rel_tol=1e-8) or not math.isclose(values[-1], number(row, "maximum"), rel_tol=1e-8):
            raise ValueError(f"CSV min/max differs from samples: {key}")
    return report, case_map, timing_map, memory_map, build_map, {name: sha(path) for name, path in paths.items()}


def workload_names(family, case, timing_map):
    if FAMILIES[family]["workloads"]:
        return FAMILIES[family]["workloads"]
    thresholds = case["case"]["thresholds"]
    return [f"{kind}:t={threshold}" for threshold in thresholds for kind in ("full", "medium_64k", "small_1k")]


def workload_label(name):
    if ":t=" in name:
        kind, value = name.split(":t=")
        return {"full": "Full", "medium_64k": "64K", "small_1k": "1K"}[kind] + "\n≥ " + f"{int(value):,}"
    return WORKLOAD_LABELS[name]


def ratio_text(value):
    if value >= 1000:
        return f"{value:,.0f}×"
    if value >= 100:
        return f"{value:.0f}×"
    if value >= 10:
        return f"{value:.1f}×"
    if value >= 0.1:
        return f"{value:.2f}×"
    return f"{value:.3f}×"


def bytes_text(value):
    if value >= 2**20:
        return f"{value/2**20:.2f} MiB"
    return f"{value/2**10:.1f} KiB"


def time_text(value):
    if value == 0:
        return "0*"
    if value >= 1000:
        return f"{value/1000:.2f}s"
    if value >= 1:
        return f"{value:.2f}ms"
    if value >= .001:
        return f"{value*1000:.1f}µs"
    return f"{value*1000000:.0f}ns"


def clean_axis(axis):
    axis.set_facecolor(IVORY)
    for spine in axis.spines.values():
        spine.set_visible(False)
    axis.tick_params(length=0, colors=MUTED, labelsize=10)
    axis.set_axisbelow(True)


def cpu_label(report):
    machine = report.get("frozen_config", {}).get("machine", "")
    for line in machine.splitlines():
        if line.startswith("machdep.cpu.brand_string:"):
            return line.split(":", 1)[1].strip()
    return report.get("frozen_config", {}).get("platform", "Recorded benchmark host")


def make_figure(family, data, output):
    report, cases, timings, memory, builds, hashes = data
    spec = FAMILIES[family]
    methods, labels = spec["methods"], spec["labels"]
    corpus = spec["cases"]
    all_ratios = []
    max_memory_fraction = 1
    expected_count = 0
    for name in corpus:
        case = cases[family, name]
        actual_methods = {m.get("method", m.get("name")) for m in case["methods"]}
        if actual_methods != set(methods):
            raise ValueError(f"Incomplete methods in {family}/{name}: {actual_methods}")
        workloads = workload_names(family, case, timings)
        actual_cells = [key for key in timings if key[:2] == (family, name)]
        if len(actual_cells) != len(methods)*len(workloads):
            raise ValueError(f"Unexpected timing cell count for {family}/{name}")
        baseline_memory = number(memory[family, name, spec["baseline"]], "accounted_working_bytes")
        for method in methods:
            max_memory_fraction = max(max_memory_fraction, number(memory[family, name, method], "accounted_working_bytes")/baseline_memory)
            expected_builds = 1 if family == "query" else 9
            if int(builds[family, name, method]["sample_count"]) != expected_builds:
                raise ValueError("Unexpected number of construction samples")
            for workload in workloads:
                value = number(timings[family, name, method, workload], "median")
                baseline = number(timings[family, name, spec["baseline"], workload], "median")
                all_ratios.append(value/baseline)
                expected_count += 1
    extent = max(1, math.ceil(max(abs(math.log2(r)) for r in all_ratios)))
    norm = Normalize(-extent, extent)
    cmap = LinearSegmentedColormap.from_list("operation_tradeoffs", [TEAL, IVORY, COBALT])
    n = len(corpus)
    fig = plt.figure(figsize=(19, 4.0 + 3.55*n), facecolor=IVORY)
    fig.text(.038, .966, spec["title"], size=27, color=NAVY, weight="bold", va="top")
    fig.text(.038, .930, spec["subtitle"], size=13, color=MUTED, va="top")
    fig.text(.970, .966, "PAGECODEC-LAB / MEASURED OPERATIONS", size=9.5, color=MUTED, ha="right", va="top")
    fig.text(.970, .945, f"{cpu_label(report)} · single thread", size=11, color=NAVY, ha="right", va="top")
    heat_left, heat_width = .166, .427
    mem_left, mem_width = .666, .105
    build_left, build_width = .861, .105
    fig.text(heat_left, .888, f"MEDIAN QUERY TIME / {spec['baseline_label'].upper()}", size=10.5, color=NAVY, weight="bold")
    fig.text(mem_left, .888, "WORKING STORAGE", size=10.5, color=NAVY, weight="bold")
    fig.text(build_left, .888, "BUILD COST", size=10.5, color=NAVY, weight="bold")
    cbax = fig.add_axes([heat_left, .859, heat_width, .012])
    colorbar = fig.colorbar(plt.cm.ScalarMappable(norm=norm, cmap=cmap), cax=cbax, orientation="horizontal")
    colorbar.set_ticks([-extent, 0, extent], labels=[ratio_text(2**-extent), "1.00×", ratio_text(2**extent)])
    colorbar.ax.tick_params(length=0, labelsize=9, colors=MUTED, pad=3)
    colorbar.outline.set_visible(False)
    fig.text(heat_left, .837, "Faster", color=TEAL, size=9.5, va="top")
    fig.text(heat_left+heat_width, .837, "Slower · logarithmic color scale", color=COBALT, size=9.5, va="top", ha="right")
    fig.text(mem_left, .854, "Capacity / baseline", size=10, color=MUTED)
    fig.text(build_left, .854, "Median · min–max", size=10, color=MUTED)
    row_top, row_bottom = .788, .170
    stride = (row_top-row_bottom)/n
    plot_height = stride*.69
    for index, name in enumerate(corpus):
        case = cases[family, name]
        descriptor = case["case"]
        top = row_top-index*stride
        bottom = top-plot_height
        count_label = f"{descriptor['rows']:,} evaluation rows" if family != "strings" else f"{descriptor['objects']:,} objects"
        fig.text(.038, top+.025, CASE_LABELS[name], size=15, color=NAVY, weight="bold", va="center")
        fig.text(.038, top+.009, count_label, size=9.5, color=MUTED, va="center")
        workloads = workload_names(family, case, timings)
        ratios = np.array([[number(timings[family, name, method, workload], "median")/
                            number(timings[family, name, spec["baseline"], workload], "median")
                            for workload in workloads] for method in methods])
        axis = fig.add_axes([heat_left, bottom, heat_width, plot_height])
        clean_axis(axis)
        axis.imshow(np.log2(ratios), cmap=cmap, norm=norm, aspect="auto", interpolation="nearest")
        axis.set_yticks(range(len(methods)), labels, fontsize=11.5, color=NAVY)
        axis.tick_params(axis="y", pad=12)
        axis.set_xticks(range(len(workloads)), [workload_label(w) for w in workloads], fontsize=9.2 if family=="query" else 11)
        axis.tick_params(axis="x", pad=8)
        axis.set_xticks(np.arange(-.5, len(workloads), 1), minor=True)
        axis.set_yticks(np.arange(-.5, len(methods), 1), minor=True)
        axis.grid(which="minor", color=IVORY, linewidth=2.4)
        axis.tick_params(which="minor", length=0)
        for y, method in enumerate(methods):
            for x in range(len(workloads)):
                value = ratios[y,x]
                color = "white" if abs(math.log2(value))/extent > .50 else NAVY
                axis.text(x,y,ratio_text(value),ha="center",va="center",fontsize=10.5 if family=="query" else 12.5,
                          color=color,weight="bold" if method==spec["baseline"] else "normal")
        if family == "query":
            for x in (2.5,5.5):
                axis.axvline(x,color=NAVY,alpha=.30,linewidth=1.1)
        ma = fig.add_axes([mem_left,bottom,mem_width,plot_height])
        clean_axis(ma)
        baseline_memory = number(memory[family,name,spec["baseline"]], "accounted_working_bytes")
        values = [number(memory[family,name,m], "accounted_working_bytes") for m in methods]
        fractions = [v/baseline_memory for v in values]
        colors = [BASE if m==spec["baseline"] else TEAL if f<=1 else COBALT for m,f in zip(methods,fractions)]
        ma.barh(range(len(methods)),fractions,height=.55,color=colors,zorder=3)
        ma.axvline(1,color=NAVY,linestyle=(0,(3,3)),linewidth=.8,alpha=.65,zorder=4)
        ma.set_ylim(len(methods)-.5,-.5)
        ma.set_yticks([])
        ma.set_xlim(0,max_memory_fraction*1.08)
        if max_memory_fraction < 1.75:
            ma.set_xticks([0,.5,1])
        else:
            ma.xaxis.set_major_locator(MaxNLocator(nbins=3,min_n_ticks=2,integer=True))
        ma.xaxis.set_major_formatter(FuncFormatter(lambda v,_: f"{v:g}×"))
        ma.grid(axis="x",color=GRID,linewidth=.8)
        for y,value in enumerate(values):
            ma.text(1.04,y,bytes_text(value),transform=ma.get_yaxis_transform(),fontsize=9.5,color=NAVY,va="center")
        ba = fig.add_axes([build_left,bottom,build_width,plot_height])
        clean_axis(ba)
        values = [builds[family,name,m] for m in methods]
        lo = min(number(v,"minimum_ms") for v in values)
        hi = max(number(v,"maximum_ms") for v in values)
        if lo == 0:
            ba.set_xscale("symlog",linthresh=.001,linscale=.5)
            ba.set_xlim(0,hi*3.0)
            ba.set_xticks([v for v in [0,.01,1,100,10000] if v<=hi*3.0])
        else:
            ba.set_xscale("log");ba.set_xlim(lo/2.5,hi*3.0)
        ba.set_ylim(len(methods)-.5,-.5);ba.set_yticks([])
        ba.xaxis.set_minor_locator(NullLocator())
        ba.xaxis.set_major_formatter(FuncFormatter(lambda v,_: f"{v:g}"))
        ba.tick_params(axis="x",labelsize=9)
        ba.grid(axis="x",color=GRID,linewidth=.8)
        ba.set_xlabel("ms · symlog (<1µs linear)" if lo==0 else "Milliseconds · log scale",fontsize=8.3,color=MUTED,labelpad=8)
        for y,(method,value) in enumerate(zip(methods,values)):
            median=number(value,"median_ms");minimum=number(value,"minimum_ms");maximum=number(value,"maximum_ms")
            color=BASE if method==spec["baseline"] else NAVY
            ba.hlines(y,minimum,maximum,color=color,linewidth=2,alpha=.65)
            ba.scatter([median],[y],s=29,color=color,edgecolor=IVORY,linewidth=.6,zorder=4)
            ba.annotate(time_text(median),(median,y),xytext=(0,8),textcoords="offset points",ha="center",va="bottom",fontsize=8.5,color=NAVY)
    fig.text(.038,.105,
             "Query cells: ratio of nine-trial medians; lower is faster. Baseline = 1.00×. Full timing samples and observed ranges are retained in CSV.",
             size=10.6,color=MUTED)
    fig.text(.038,.083,spec["memory_note"]+" These are accounted bytes, not process RSS.",size=10.2,color=MUTED)
    fig.text(.038,.061,spec["build_note"]+" Ranges are not confidence intervals.",size=10.2,color=MUTED)
    fig.text(.038,.033,"SOURCE  results/operations-2026-10-30/  ·  frozen sources, input SHA-256, protocol and all measured cases",size=9.4,color=NAVY)
    fig.text(.970,.013,f"{expected_count} timing cells · JSON SHA-256 {hashes['all-results.json'][:20]}…",size=8.5,color=MUTED,ha="right")
    description = (spec["subtitle"] + "; all methods and corpora shown. Query cells are ratios of medians of nine trials; lower is faster. "
                   + spec["memory_note"] + " Accounted storage is not RSS. " + spec["build_note"]
                   + " No confidence intervals. Source hashes: " + json.dumps(hashes,sort_keys=True))
    fig.canvas.draw()
    renderer=fig.canvas.get_renderer()
    for label in fig.texts:
        box=label.get_window_extent(renderer)
        if box.x0 < 0 or box.x1 > fig.bbox.width or box.y0 < 0 or box.y1 > fig.bbox.height:
            raise ValueError(f"Figure text extends beyond canvas: {label.get_text()}")
    output.parent.mkdir(parents=True,exist_ok=True)
    fig.savefig(output.with_suffix(".png"),dpi=190,metadata={"Title":spec["title"],"Description":description})
    svg=output.with_suffix(".svg")
    fig.savefig(svg,metadata={"Title":spec["title"],"Description":description,"Date":None})
    svg.write_text("\n".join(line.rstrip() for line in svg.read_text().splitlines())+"\n")
    plt.close(fig)
    print(f"Wrote {output.with_suffix('.png')} and {svg}")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results",type=Path,default=DEFAULT)
    parser.add_argument("--output-dir",type=Path,default=ROOT/"assets")
    args=parser.parse_args()
    plt.rcParams.update({"font.family":"DejaVu Sans","text.color":NAVY,"axes.labelcolor":MUTED,
                         "svg.hashsalt":"pagecodec-operations-v1","savefig.facecolor":IVORY})
    data=load(args.results.resolve())
    for family in FAMILIES:
        make_figure(family,data,args.output_dir/f"operations-{family}")


if __name__=="__main__":
    main()
