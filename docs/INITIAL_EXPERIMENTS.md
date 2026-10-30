# Initial experiments: complete V1 and V2 history

These are the measurements made on 30 October 2026 before the larger real numeric corpus was added. They are retained as historical results, with losing cases included. They compare an independent C page codec with three codecs from Apple's user-space Compression framework. They do not demonstrate superiority over macOS memory management or its kernel compressor, and do not change physical RAM.

V1 met the predefined strict criterion against all three Apple references in **2 of 12 nonzero inputs**. V2 met it in **2 of 20 nonzero inputs**, including **0 of its 8 newly introduced inputs**. Both winning inputs, `events_dev.bin` and `events_holdout.bin`, are synthetic records from one deliberately structured workload family. No real-world input in this initial corpus passed against all three references. The all-zero control is excluded from these headline counts.

## Frozen iterations

| Iteration | Inputs | Total input bytes | Method cases | Frozen UTC | Custom codec SHA-256 |
|---|---:|---:|---:|---|---|
| V1 | 13 | 29,625,950 | 65 | 2026-10-30 13:56:32 | `c7d8f44f8cbee7c85f8f6f6f6194d3aa3a685ba1392fde31449d4ef23f0b043d` |
| V2 | 21 | 46,403,181 | 105 | 2026-10-30 14:05:42 | `fbc88ada9bc4654241a92353cd34b872132106916820334f1c30ab2efcf826c2` |

V1 detects uniform bytes, repeated 64-bit patterns, narrow byte ranges, and four interleaved lanes of 32-bit values, storing lane bases and packed deltas. It falls back to raw storage when its representation would not be smaller. V1 source is preserved in [`versions/v1`](../versions/v1/).

After observing V1's sparse-outlier failure, V2 added a 32-bit background plus indexed exceptions. Its background estimate uses the first three words and its prefix gate examines up to 32 words, followed by a full scan. A fast sparse shortcut trades selection work against searching every alternative representation. Misleading prefixes, density changes, and unaligned layouts are explicit failure cases. The codec makes these decisions from bytes, without receiving a dataset name or schema. The root [`codec.c`](../codec.c) is the frozen V2 source.

The 13 original inputs were already observed when V2 was designed. They are regression cases for V2, not fresh blind validation. Eight new synthetic inputs were prepared for the V2 evaluation: a nonzero background with several exception densities, forced first/last-word exceptions, a one-byte shift and partial tail, alternating sparse/dense pages, and a misleading three-word prefix. V2 was frozen before those performance measurements. These cases probe one codec feature; they are not eight independent real applications. Neither iteration establishes a new scientific invention: base/delta packing and exception coding have established precedents.

The manifests preserve source URLs, byte lengths, SHA-256 digests, original dataset descriptions, and development/holdout categories. Weather CSV and weather int32 views share the same underlying observations. The int32 views contain only precipitation, maximum/minimum temperature, and wind at an exact scale of ten; dates and weather labels are omitted. Consequently, their compression ratios refer to that numeric array, not to lossless compression of the entire CSV. The small weather arrays fit in cache. The REA bundle is public software used solely as byte data; it is not executed. `previous_plot.png` is a precompressed benchmark fixture, not a project illustration or evidence of the current result.

## Measurement method

The machine was an Apple M4 Pro with 24 GiB RAM and macOS 15.1.1, using Apple clang 16.0.0. The comparator was compiled with `-O3 -Wall -Wextra -Werror` and linked with `-lcompression`. Its frozen SHA-256 is `11cc8da529d5b9449040e547bdb2e752b22aa8db5c766709b45e44281f989587`; the same [`compare.c`](../compare.c) serves both iterations.

Each input was split into 16 KiB pages, including its exact final partial page, and tested with raw copies, Apple `COMPRESSION_LZ4`, Apple `COMPRESSION_LZ4_RAW`, Apple `COMPRESSION_LZFSE`, and the custom codec. LZ4_RAW prevents the LZ4 framing overhead alone from defining success. Each method received identical input bytes.

One warmup round preceded nine paired rounds. Each round deterministically permuted method order; the seed for input index `i` was `20261030 + i`. To time small inputs, fixed batching used `min(1024, max(1, ceil(8 MiB / input_bytes)))`, with batch time divided by that count. Batching reuses the input; it does not enlarge the stored corpus. The recorded `encode_ms` and `decode_ms` are medians, and every individual sample remains in the JSON archive.

The timed operations are single-thread encode and full decode/materialization, including decisions, copies, and table writes. Allocation, disk I/O, corpus generation, and validation occur outside the timed interval. Buffers and scratch are preallocated. Every batch's reconstructed bytes were checked; all 65 V1 and 105 V2 method cases report exact verification. This is warm-buffer throughput on an active machine, not physical DRAM latency, cold-cache performance, or an application-level speedup.

Storage counts the chosen payload, the custom codec's headers where present, and a common 16-byte page table per page for each compressed method. Each compressed method retains a raw page when encoding does not reduce its payload; its page table remains. Raw copying has no page table. Source bytes, output buffers, scratch, unused reserved capacity, allocator overhead, and OS overhead are excluded. Therefore the reported size is a representation size, not measured resident memory or RAM saved by an integrated application. The original before/after aggregate VM and swap counters remain in the JSON for context; they are not attributed to this benchmark.

The predefined strict criterion against a reference is simultaneous:

- custom total storage is no larger than reference total storage;
- custom median encoding time is at most 90% of reference median encoding time;
- custom median decoding time is at most 90% of reference median decoding time.

A fast raw fallback that uses more space does not pass. This median threshold is a descriptive rule, not a statistical significance test or a guarantee that every paired trial wins. No energy, tail-latency, memory-pressure, or kernel-integration benefit was measured.

## Every result, including every failed criterion

`PASS` means all three criteria passed for that reference. A letter identifies each failed criterion: **S** = storage larger, **E** = encoding did not meet the 10% margin, **D** = decoding did not meet the 10% margin. E or D may mean a smaller improvement than required, rather than an outright slowdown. Percentages include the page table and are rounded only for this display; full measurements are unrounded in JSON/CSV.

### V1

| Input | Custom storage / input | vs Apple LZ4 | vs Apple LZ4_RAW | vs Apple LZFSE |
|---|---:|---|---|---|
| `zeros.bin` | 0.11% | PASS | PASS | PASS |
| `events_dev.bin` | 28.35% | PASS | PASS | PASS |
| `events_holdout.bin` | 39.13% | PASS | PASS | PASS |
| `events_shifted.bin` | 60.96% | PASS | PASS | S |
| `random.bin` | 100.10% | E, D | E, D | D |
| `scattered_palette.bin` | 100.10% | S, E | S, E | S |
| `sparse_outliers.bin` | 99.57% | S, E | S, E | S |
| `weather_dev.csv` | 100.13% | S | S | S |
| `weather_dev_i32.bin` | 31.57% | PASS | PASS | S |
| `weather_holdout.csv` | 100.13% | S | S | S |
| `weather_holdout_i32.bin` | 31.57% | PASS | PASS | S |
| `rea_cli_public.js` | 100.10% | S | S | S |
| `previous_plot.png` | 100.10% | S | S | S |

### V2

| Input | Custom storage / input | vs Apple LZ4 | vs Apple LZ4_RAW | vs Apple LZFSE |
|---|---:|---|---|---|
| `zeros.bin` | 0.11% | PASS | PASS | PASS |
| `events_dev.bin` | 28.35% | PASS | PASS | PASS |
| `events_holdout.bin` | 39.13% | PASS | PASS | PASS |
| `events_shifted.bin` | 60.96% | PASS | PASS | S |
| `random.bin` | 100.10% | E, D | E, D | PASS |
| `scattered_palette.bin` | 100.10% | S | S | S |
| `sparse_outliers.bin` | 0.88% | D | D | PASS |
| `weather_dev.csv` | 100.13% | S | S | S |
| `weather_dev_i32.bin` | 31.57% | PASS | PASS | S |
| `weather_holdout.csv` | 100.13% | S | S | S |
| `weather_holdout_i32.bin` | 31.57% | PASS | PASS | S |
| `rea_cli_public.js` | 100.10% | S | S | S |
| `previous_plot.png` | 100.10% | S | S | S |
| `new_sparse_01pct.bin` | 0.30% | E, D | E, D | PASS |
| `new_sparse_5pct.bin` | 9.01% | PASS | E | PASS |
| `new_sparse_20pct.bin` | 44.32% | S | S | S |
| `new_sparse_50pct.bin` | 100.10% | S | S | S |
| `new_sparse_1pct_first_outlier.bin` | 3.25% | D | D | S |
| `new_sparse_shifted_tail.bin` | 3.87% | S, E, D | S, E, D | S |
| `new_alternating_sparse_dense.bin` | 50.88% | S | S | S |
| `new_misleading_prefix.bin` | 100.10% | S, E | S, E | S |

Sparse exceptions substantially reduce V2's representation size for some cases, but do not yield a strict win against all references on any new V2 case. The misleading prefix defeats its sparse candidate selection; scattered alphabets and text favor other compression families; LZFSE often produces smaller representations. High-entropy and already compressed inputs provide no basis for assuming useful compression. These failures are part of the experiment, not exclusions from the corpus.

## Archive integrity and path redaction

Each iteration has [`all-results.json`](../results/initial-v1/all-results.json), `all-results.csv`, and `frozen-config.json` under [`results/initial-v1`](../results/initial-v1/) or [`results/initial-v2`](../results/initial-v2/). JSON retains all sample arrays, method orders, checksums, source hashes, machine/OS versions, and exact verification flags. CSV files are byte-for-byte copies of the original measurement exports. The manifests are [`initial-v1.json`](../manifests/initial-v1.json) and [`initial-v2.json`](../manifests/initial-v2.json).

The published JSON replaces workstation-specific dataset/input paths with `data/initial/<name>` and removes the compiler's `InstalledDir` line. The original manifests included private absolute paths, so the public manifest files necessarily have different hashes. Each frozen configuration, including the copy embedded in `all-results.json`, records:

- `original_manifest_sha256`: the historical digest recorded before measurement;
- `published_manifest_sha256`: the digest of the corresponding public manifest;
- `path_redaction_note`: the exact scope of the publication edits.

The legacy `manifest_sha256` field is retained with its original value and identifies the original private-path manifest, not the public file. No timing, size, input bytes, source bytes, source digest, or trial order was changed when packaging these results. Original executable hashes are provenance records; rebuilding in a different directory or toolchain need not reproduce those binary hashes. Source and corpus hashes provide the portable identity checks.

## Recreate inputs or repeat a measurement

From the repository root, Python 3.9+ and curl reproduce all 21 inputs into `data/initial` and verify every size and SHA-256 against the public V2 manifest:

```sh
python3 scripts/prepare_initial.py
```

The generator uses the original deterministic seeds and layouts, fetches weather from a pinned repository commit, and reads the pinned REA npm tarball without installation or execution. Downloads are cached in `data/downloads`; the expected manifests are never overwritten. V1's 13 files are a byte-identical subset of these generated inputs.

On macOS with the Command Line Tools, a new V2 measurement can be written to a separate directory:

```sh
python3 scripts/benchmark.py --manifest manifests/initial-v2.json --output measurements/initial-v2-rerun
```

The runner rejects a nonempty destination, preserving the archived runs. Do not interpret a fresh run as reproducing historical timing exactly: system activity, cache state, scheduling, and toolchain changes can affect it. To build the historical V1 comparator without replacing the root V2 codec:

```sh
mkdir -p build
clang -O3 -Wall -Wextra -Werror compare.c versions/v1/codec.c -lcompression -o build/compare-v1
```

Its inputs and per-input seeds are recorded in the V1 manifest and results. The portable codec correctness suite can also run without Apple frameworks using `make test sanitize`; Apple comparison measurements require macOS.
