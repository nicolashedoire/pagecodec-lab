# Exact queries over compressed integer columns

This standalone experiment tests whether lossless representations help complete
queries, rather than only making a whole-buffer decoder faster. It does not
modify the frozen page codec or the operating system. The implementation is a
small research prototype, not a new compression principle or a substitute for
the full FastLanes implementation.

```sh
c++ -std=c++17 -O3 -Wall -Wextra -Werror \
  experiments/query/query_bench.cpp -o build/query-bench
build/query-bench --x data/example-x.i32 --y data/example-y.i32 \
  --thresholds 10,50,100 --repetitions 9 --seed 20261030 \
  --output measurements/query.json
python3 -m unittest discover -s tests -p test_query.py -v
```

Use GCC or Clang. AArch64 builds use explicit NEON for the raw baseline; other
platforms use a compiler-optimizable loop. Both inputs contain headerless
little-endian signed int32 values with the same nonzero row count, at most
16,777,216 rows per column. `INT32_MIN` is the missing sentinel, so that bit
pattern cannot simultaneously represent an observed measurement.

## Query contract

Each query specifies an arbitrary half-open row window `[begin,end)` and an
int32 threshold. It returns:

- `count`: rows with a nonmissing `x` satisfying `x >= threshold`.
- `sum`: the exact int64 sum of nonmissing `y` among those rows. A missing `y`
  does not remove the row from `count`. The empty sum is zero.

This is an integer benchmark contract; SQL would typically report NULL for an
empty SUM. Inputs remain immutable during a run. Integer reconstruction uses
int64 arithmetic, and the enforced input limit keeps all aggregate sums and
intermediates within int64. No floating-point values, NaNs, infinities or signed
zeros are accepted by this format. Supporting floating-point arrays would need
a separate bit-exact contract and tests.

## Methods

All representations preserve the same two columns and missingness. Blocks
contain 1,024 rows, with a short final block and zero padding only in the final
storage words; padded rows never participate in queries.

| Method | Representation and execution |
|---|---|
| `raw` | Original int32 arrays plus block min/max index. AArch64 uses four-lane NEON comparison with widening int64 accumulation. |
| `for` | Per-column block minimum, exact unsigned residuals packed horizontally, and validity bitmaps. Decode only values required by the filter/aggregate; no full output buffer. |
| `bitplane_eager` | Same residuals stored as bitplanes. Compare all predicate planes with bitwise masks, then compute the masked sum using population counts. “Eager” means no early termination within the residual comparison, not scalar reconstruction of every value. |
| `bitplane_progressive` | Same bitplane representation, but stop comparing when every candidate is decided or all remaining threshold bits are zero. |
| `bitplane_adaptive` | Use a per-block 16-bin high-residual histogram. Choose progressive comparison when the threshold bin contains at most one eighth of the block's rows; otherwise choose eager comparison. |
| `rle` | Run-length encode consecutive identical `(x,y)` pairs, including missing sentinels. Binary-search run starts and aggregate intersecting run lengths. |

Raw, FOR and bitplane methods can reject blocks whose valid maximum is below
the threshold; their index/header costs are included. RLE tests its run values
directly. There are no global cached query answers or per-threshold indexes in
any measured representation. RLE is a pair-run baseline, not a complete
columnar RLE engine. FOR is a straightforward reference, not the optimized
FastLanes SIMD implementation. A win over either alone cannot establish a win
over state-of-the-art column stores.

The adaptive rule, four high histogram bits, one-eighth cutoff and block size
are fixed in source. The histogram describes actual block contents, not
benchmark answers. Its construction time, vector capacity and metadata bytes
are charged to the adaptive method. Small blocks may make its cutoff zero.

## Exactness argument

A valid integer is stored as `x = base + residual`. The residual is nonnegative
and at most `UINT32_MAX`; all arithmetic around the signed base is widened.
After reading a high-bit prefix `h` of a width-`w` residual, with `k` bits read,
the value belongs to:

```text
L = base + (h << (w-k))
U = min(L + 2^(w-k) - 1, block_maximum)
```

`L >= threshold` certifies true and `U < threshold` certifies false. The
implementation maintains bitmasks for greater-than and equal-prefix candidates
while reading planes from most to least significant. A candidate that becomes
greater stays accepted; when no equal candidates remain, lower bits cannot
change any answer. If the remaining threshold suffix is zero, all equal-prefix
candidates also qualify. Eager evaluation computes the same comparison while
visiting every predicate plane. All low data bits remain stored.

For the resulting predicate mask `M`, remove missing `y` values to obtain `S`.
Writing `y_i = ybase + sum_j 2^j B_ij` gives the exact identity:

```text
sum_selected_y = ybase * popcount(S)
                 + sum_j 2^j * popcount(S AND y_bitplane_j)
```

The independent raw oracle checks the final count and sum. Tests cover negative
bases, full-width residuals, extreme signed values, missingness, zero-width
blocks, partial words/blocks, single rows, empty windows and runs crossing block
boundaries. The algebra explains correctness, not speed or novelty.

## Schedule, timing and provenance

Prefer explicit `--thresholds` calibrated on separate development rows. If
omitted, the program chooses p10/p50/p90 from valid `x` in the first
`ceil(rows/5)` rows. Those rows remain queryable, and the JSON states this
fallback; it is not a held-out evaluation. No parameter adapts to measured
performance or recognizes dataset filenames.

For each threshold the schedule contains one full scan, four reproducible
windows of up to 65,536 rows, and sixteen windows of up to 1,024 rows. Windows
are clamped to available rows; on tiny inputs some queries coincide. These are
repeated operations on actual data, not repeated corpus bytes. The RNG uses
`mt19937_64` and explicit modulo-based selection for reproducibility.

One warm-up per workload/method precedes nine measured paired rounds by
default. Each round shuffles method order. The timer covers a complete schedule
of queries and an answer checksum. For small schedules, fixed batching repeats
the schedule up to 64 times, targeting approximately 1 MiB of logical raw input;
reported times divide by this batch count. JSON retains all samples and orders.
Construction is measured once, separately. Input I/O, allocation during input
loading, calibration, exact oracle checks and JSON output are outside query
timing. These are warm repeated queries without CPU affinity or frequency
control, not proof that a workload is DRAM-bound.

Before timing, every stored column value is reconstructed and compared with
its original, including missingness, without retaining a full output array.
Every method must then match an independent scalar oracle for all
scheduled queries plus boundary and reproducible randomized windows. Every
timed schedule must also produce the expected answer checksum. `--validate-only`
performs correctness checks without timing; its timing arrays are empty. Input
FNV-1a hashes identify bytes for diagnostics; the external runner should record
cryptographic SHA-256 and source/compiler/machine provenance.

## Memory accounting and isolated probes

`payload_bytes` includes packed values and validity bitmaps. `metadata_bytes`
includes block headers, indexes, RLE start/length pairs, and adaptive histograms
where applicable. Their sum is `representation_bytes`.
`allocated_capacity_bytes` additionally counts unused vector capacities and
the owning C++ object/vector headers. `explicit_scratch_bytes` includes the
reusable 128-byte bitmask array used for bitplane aggregation.
`owned_working_bytes` is capacity plus this explicit scratch. Allocator headers,
compiler-generated stack frames, executable mappings and runtime overhead are
not included. This is accounting, not a measurement of RSS.

An all-method run retains the raw oracle and several representations together;
the three bitplane methods share one physical bitplane store. Per-method
footprints describe separate ownership scenarios and must not be added to infer
process RSS. The query schedule and precomputed oracle answers are benchmark
harness storage, not cached answers used by any query algorithm.

For process-level probes, select one method:

```sh
/usr/bin/time -l build/query-bench --x data/example-x.i32 --y data/example-y.i32 \
  --thresholds 10,50,100 --only-method bitplane_progressive --repetitions 1 \
  --output measurements/query-isolated.json
```

An isolated non-raw run constructs only its selected representation, validates
it, retains the small oracle-answer schedule, and releases the source arrays
before timing. JSON reports this and remaining input capacity. The peak RSS
reported by `time` still includes construction/validation, when source and
representation coexist, and does not establish the steady-state live footprint
or RAM freed system-wide. A raw-only run necessarily retains its source arrays.

## JSON shape

The root object has `schema_version: 1`, `rows`, `block_rows`, `seed`,
`repetitions`, input names/hashes, threshold/semantic/scope descriptions,
`verified`, `validation_cases`, and source-retention fields.

`methods[]` contains `method`, `construction_ms`, `payload_bytes`,
`metadata_bytes`, `representation_bytes`, `allocated_capacity_bytes`,
`explicit_scratch_bytes`, `owned_working_bytes`, and `model_metadata_bytes`.

`workloads[]` contains `kind` (`full`, `medium_64k`, `small_1k`), `threshold`,
`logical_rows_per_schedule`, `inner_iterations`, exact `queries[]` with
`begin/end/count/sum`, `trial_order[][]`, and `results[]` with `method`,
`verified`, `median_ms` and `samples_ms`. Throughput computed from logical rows
does not describe physical bytes actually read by compressed execution.
