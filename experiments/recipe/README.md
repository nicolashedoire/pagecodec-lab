# Experiment B: keep a recipe instead of derived arrays

This application experiment asks when it is cheaper to recompute deterministic integer features than to retain their values. It compares five representations of the same eight derived columns. Every method retains both original input arrays because the application still needs them. It does not intercept memory allocations, change macOS, or replace Apple's memory compressor.

The practical idea is avoiding intermediate materialization. Matrix-free numerical methods and database query execution already use this principle. This experiment tests a specific feature pipeline and access schedule; it does not claim a new general principle or algorithm.

## Build and run

From the repository root, using a C11 compiler and a C++17 compiler:

```sh
mkdir -p build
cc -O3 -std=c11 -Wall -Wextra -Werror -c codec.c -o build/recipe-codec.o
c++ -O3 -std=c++17 -Wall -Wextra -Werror experiments/recipe/recipe_bench.cpp build/recipe-codec.o -o build/recipe-bench
python3 tests/test_recipe.py --binary build/recipe-bench

build/recipe-bench \
  --x data/operations/household-x.i32 \
  --y data/operations/household-y.i32 \
  --repetitions 9 --seed 20261030 --tile-rows 256 --cache-tiles 64 \
  --output build/recipe-household.json
```

The [operations manifest](../../manifests/operations-v1.json) pins the real source projections and their hashes. The household pair contains scaled active/reactive power; the Covertype pair contains elevation/distance to roadways. Evaluation uses the final 80% of each source in its original order. Access schedules are synthetic; data rows are neither duplicated nor padded. Input files must be equally sized, nonempty little-endian signed int32 arrays, each at most 32 MiB. This implementation targets a 64-bit process. Values are read exactly, without floating-point conversion.

`--queries` controls sparse point reads (default 4096). Hot windows use up to 256 requests and changing hot sets use up to 1024. `--only-method NAME --repetitions 1` constructs just one method and is useful for a separate process peak-memory measurement, for example on macOS:

```sh
/usr/bin/time -l build/recipe-bench \
  --x data/operations/household-x.i32 --y data/operations/household-y.i32 \
  --only-method recipe_cache --repetitions 1 --output build/recipe-cache-rss.json
```

Use nine-trial runs for performance comparisons. A one-trial resident-memory probe is a different measurement. On Linux, `/usr/bin/time -v` reports peak resident memory. The program's `process_peak_rss_bytes` is a process-lifetime high-water mark; in a combined run it cannot be assigned to an individual method.

## Exact feature contract

| Column | Integer expression |
|---|---|
| `sum` | `x + y` |
| `difference` | `x - y` |
| `product` | `x * y` |
| `x_squared` | `x * x` |
| `y_squared` | `y * y` |
| `minimum` | `min(x, y)` |
| `maximum` | `max(x, y)` |
| `absolute_difference` | `abs(x - y)` |

These are ordinary arithmetic, interaction and range features, not a claim that all eight improve a predictive model. Units follow the expressions: squared and interaction features have squared source units when the two inputs share a unit. No rounding or rescaling is performed.

Input `INT32_MIN` means missing. If either input is missing, all eight output features are `INT64_MIN`. Otherwise both operands lie in `[-2147483647, 2147483647]`; converting them to int64 **before** arithmetic makes every listed expression safe. Products and squares are at most `2147483647²`, below `INT64_MAX`; differences are at most 4294967294. No valid result collides with the output missing sentinel.

The recipe is deterministic because it uses these fixed integer operations on retained, unmodified inputs, with no time, random state, external state or floating-point reductions. Immutability alone would not establish determinism for an arbitrary application. Updating a source would require explicit invalidation or versioning; updates are outside this experiment.

## Representations and operations

| Method | Retained derived representation | Access behavior |
|---|---|---|
| `materialized` | Eight complete int64 columns | Direct reads |
| `compressed_blocks` | Independent feature tiles compressed by the frozen root codec; block index | Decode the requested tile into a one-tile scratch buffer |
| `recipe` | Eight operation IDs and one tile buffer | Compute a tile when it differs from the last accessed tile |
| `recipe_cache` | Recipe plus a bounded direct-mapped tile cache and tags | Compute only on a cache miss |
| `recipe_direct` | Recipe only; no derived buffer | Compute only requested scalar features, directly into checksum consumption |

A default tile holds 256 rows × 8 features × 8 bytes = 16 KiB. Within a tile each feature is contiguous. The final tile contains only actual rows. The recipe cache has 64 full-capacity slots (1 MiB); slot selection is `tile_index % cache_tiles`. The direct variant uses the same traversal and checksum as the other methods but computes only the requested feature at each consumed row, without allocating a derived tile. Its tile hit/miss counters are zero because no tile lookup occurs. Compression is byte-exact for the native int64 representation in this process; encoded blocks are not a cross-platform interchange format.

The four workloads are:

- `sequential_all`: one pass over all eight features and all rows.
- `repeated_hot_windows`: cycle through at most 16 early tiles, constrained to one quarter of the cache slots, up to 256 requests. Every feature is consumed.
- `sparse_points`: seed-generated individual row/feature reads. A miss in a tile-based method still computes or decodes a **whole tile**, exposing over-computation. `recipe_direct` instead computes one feature for the requested row, providing a fair alternative to that deliberately simple tiled design.
- `changing_hotset`: visit four successive sets of `cache_tiles` tiles repeatedly, up to 1024 requests. The sets conflict in the direct-mapped cache. If the input has too few tiles, wraparound reduces this adversarial effect; this is visible in reported hits and misses.

All methods receive the same schedules. Feature values feed a deterministic unsigned 64-bit checksum incorporating row and feature identity. The source oracle is streamed and does not allocate derived columns. Every compressed block is additionally decoded and checked value by value. The Python tests independently reproduce the sequential checksum with arbitrary-precision integer arithmetic, including missing values, opposite int32 limits, and incomplete tiles. Checksums are error detectors, not cryptographic proofs.

## Timing, memory and amortization

Nine default trials shuffle method order with a reproducible Fisher–Yates schedule. A method is constructed, measured and destroyed before the next method starts. Workload order is fixed as listed above; logical tile caches are cleared before each workload. Hardware caches and the allocator are not flushed. There is no separate warmup, and all nine samples are retained.

Construction includes allocation and creation of the representation. Query timing includes tile lookup, decoding/recomputation and checksum consumption. Input I/O, schedule generation, streamed oracle evaluation, per-value codec validation, logical-cache reset and JSON serialization are excluded. Both elapsed wall time and process CPU time are recorded. CPU time is not an energy measurement, and these experiments are single-threaded.

Memory accounting includes actual vector capacities for both source arrays, derived payloads, block indexes/cache tags, scratch/cache buffers, recipe IDs, C++ object metadata and the common query schedule. `derived_stored_bytes` reports logical payload size separately. `retained_accounted_bytes` is the sum of non-overlapping capacity components. `construction_accounted_peak_bytes` adds the temporary encoded buffer and its vector object; it is an allocation-accounting estimate, not a resident-memory measurement. Allocator bookkeeping, code, libraries and report storage are excluded from this accounting and can affect process RSS.

The JSON includes an illustrative crossover against materialization:

`total wall time = construction median + executions × workload median`.

An execution means the **entire named workload**, not one scalar query. A positive crossing is the point where this linear estimate changes which method is faster. A missing crossing does not imply universal dominance: inspect both construction and per-workload differences. This calculation uses medians, has no confidence guarantee, and assumes repeated executions have the measured cost. Real applications can have different access mixes, hardware-cache behavior, concurrency and update costs.

## JSON structure

`schema_version: 1` reports input names/fingerprints/row counts, configuration and semantics, workload sizes and expected checksums, and `methods[]`. Each method contains:

- `memory`: source, payload, index, cache, scratch and metadata byte counts.
- `construction_median_wall_ms`, `construction_median_cpu_ms`, and every construction sample with repetition/order position.
- `workloads[]`: median wall/CPU milliseconds, matching checksum, and all timing/hit/miss samples.

`amortization_vs_materialized[]` contains construction and per-workload time differences plus the estimated crossing when nonnegative. A method-only run leaves this comparison empty. Source fingerprints are FNV-1a diagnostics; use manifest SHA-256 values for corpus identity. Reports contain no raw source rows or absolute input paths.

The benchmark cannot establish a universal memory win. Tiny sources can be smaller than a fixed cache; point reads can make tile recomputation expensive; frequently reused results can favor materialization. The intended outcome is an explicit, reproducible tradeoff between retained bytes and work performed.
