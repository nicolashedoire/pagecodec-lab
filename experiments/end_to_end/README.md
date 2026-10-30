# Offline analytical report: source text to complete answers

This is a standalone command-line application. Given a source dataset and a
supplied list of row-window/threshold questions, it writes an exact JSON report.
The benchmark measures the whole fresh process, including text parsing,
representation construction, all answers and output. It does not measure only
prebuilt, warmed query operators.

The application computes `count(valid x >= threshold)` and the sum of nonmissing
`y` among those selected rows. Row windows are half-open `[begin,end)` in the
evaluation partition. All arithmetic is exact: int32 inputs, int64 sums, with
`INT32_MIN` reserved for missing values. Empty selections produce count and sum
zero. For forest observations the two columns are elevation and distance to
roadways; for household readings they are active/reactive power scaled by 1000;
for bicycle rentals they are total and registered counts.

## Run a real report

From the repository root:

```sh
make end-to-end CXX=clang++
python3 scripts/prepare_end_to_end.py
build/report-bench \
  --input data/end-to-end/covertype.csv --format covertype \
  --rows 464810 --skip-rows 116202 \
  --queries data/end-to-end/covertype-4096.tsv \
  --method stream_sweep --output build/forest-report.json
```

The TSV query plan has no header: each line contains `begin`, `end`, and
`threshold`, separated by tabs. For example, `0<TAB>464810<TAB>2903` asks about
all evaluation rows whose elevation is at least 2903 metres. The output retains
each original query and its count/sum, in original request order. Replace the
plan to ask different questions. The prototype targets three pinned unquoted
source formats, not arbitrary CSV dialects. Inputs must have the stated row
count after skipping; limits are 16,777,216 evaluation rows and 65,536 requests.

## Five strategies for the same offline task

| Method | Retained data and execution |
|---|---|
| `raw` | Two int32 columns, block min/max, valid counts and sums; native ARM NEON scans where a block cannot be accepted/rejected wholesale |
| `bitplane` | Streamed 1024-row blocks of exact residual bitplanes and validity masks; progressive comparison and popcount aggregation |
| `bitplane_adaptive` | Same representation plus 16-bin histograms; fixed one-eighth rule selects eager/progressive comparison |
| `sorted_blocks` | Original columns for partial windows plus sorted valid x keys and prefix y sums within each block; exact suffix lookup for full blocks |
| `stream_sweep` | No retained row columns: sorted query endpoints and a Fenwick histogram over threshold bins; answer range sums/counts during source ingestion |

Every indexed representation has the same inexpensive full-block all-match
count/sum shortcut. This makes the raw control stronger than in the earlier
operation microbenchmark. The bitplane implementations construct exactly sized
payloads one block at a time, with an 8 KiB input chunk; they never coexist with
complete uncompressed input columns. The sorted index intentionally pays extra
space for faster conditional aggregates. All of these costs are measured.

The sweep is an essential control because this is an **offline report**: every
question is known before reading the dataset. Sort each query's begin/end row
events. As records arrive, assign valid x values to the bin found by an upper
bound over the sorted distinct query thresholds; update count and y-sum Fenwick
trees. At an endpoint, obtain the suffix aggregates for that query's threshold.
Subtract its begin snapshot from its end snapshot. With Q requests and K distinct
thresholds, construction costs O(Q log Q); source processing and endpoints cost
O((N+Q) log(K+1)), and additional state is O(Q+K). There are no retained source
rows and no approximation. Fenwick trees and offline sweeps are established
techniques; this application does not claim to invent them.

The sweep cannot answer a new, unplanned question after discarding the source
without another read. This is a capability difference from an interactive store,
but it satisfies the full supplied report contract. Hiding that control would
make a comparison against retained arrays artificially weak.

## Measurement and correctness

`make test-end-to-end sanitize-end-to-end CXX=clang++` runs independent Python
oracle tests, including all 33 residual widths, signed limits, missingness,
partial blocks, full-block shortcuts, empty windows, endpoint ordering and all
three source formats. Sanitizers check address and undefined-behavior errors.

The full runner reads a frozen manifest and launches nine fresh processes per
method, dataset and query-count level, with shuffled order within each paired
round. Each process reads the supplied query file and every original source
record, parses the two selected fields (including discarded calibration records), builds its chosen structures, computes
the complete report and writes JSON. A common FNV-1a integrity pass over the
projected values is also timed. The parent verifies every answer against a
separate Python bin-histogram implementation, and verifies the projection
fingerprint against the independent prior numeric conversion. The oracle is
never resident in the measured child.

External elapsed time uses the parent's monotonic high-resolution clock around
`/usr/bin/time` plus the child, including launch and exit. Peak resident set size
comes from `time -l` on macOS, or `time -v` on Linux (KiB converted to bytes).
No wrapper overhead is subtracted. Output serialization and file close count,
but no `fsync` durability is promised. Report outputs are actual query answers,
not a synthetic checksum-only consumer.

Internal `timing_ms` fields are diagnostics. Sweep query work is fused into
ingestion; its post-ingestion query duration is not a comparable standalone
query metric. `rss_after_ingestion_bytes` is a process-lifetime high-water mark
observed at that stage, not instantaneous RSS. Capacity estimates omit allocator
and runtime overhead; the primary memory metric is the external whole-process
peak, not those estimates.

Archive download/extraction, threshold calibration, query-plan generation,
input hashing and the independent oracle are preparation, outside the measured
application. The task begins with raw text and a supplied query plan. File caches
are not flushed; hash verification warms them. The plans are generated,
immutable and unique, not captured production or interactive traces.

[Frozen workload and pass criterion](../../manifests/end-to-end-v1.json) ·
[Complete end-to-end results](../../docs/END_TO_END.md).
