# Benchmark methodology

This experiment compares lossless representations of process-owned byte buffers.
It measures the cost of encoding and fully reconstructing those buffers on one
machine. The executable implementation is [compare.c](../compare.c); the runner
is [scripts/benchmark.py](../scripts/benchmark.py).

## What is compared

| Result name | Implementation |
|---|---|
| `raw` | A complete `memcpy` for encoding and another for decoding |
| `apple_lz4` | Apple Compression buffer API, `COMPRESSION_LZ4` |
| `apple_lz4_raw` | Apple Compression buffer API, `COMPRESSION_LZ4_RAW` |
| `apple_lzfse` | Apple Compression buffer API, `COMPRESSION_LZFSE` |
| `custom_rc` | This repository's frozen V2 `rc_encode` / `rc_decode` |

The Apple entries call the public macOS
[`compression_encode_buffer` and `compression_decode_buffer` APIs](https://developer.apple.com/documentation/compression).
They link the system Compression library and are not a measurement of the
operating system's memory compressor. Apple
documents [framed LZ4](https://developer.apple.com/documentation/compression/compression_lz4)
and [raw LZ4 blocks](https://developer.apple.com/documentation/compression/compression_lz4_raw)
separately; including both avoids making the custom format's comparison depend
only on the framed API's overhead.

The kernel has additional algorithms and policy. For example, Apple's published
[XNU compressor source](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/vm/vm_compressor_algorithms.c)
describes a WKdm/LZ4 adaptive scheme. Reading that source does not establish
which path this machine's running kernel takes. This project does not hook that
kernel path, inspect other processes, or change physical DRAM.

The custom codec combines repeated-value modes, fixed-width base/delta lanes,
and sparse exceptions; see [FORMAT.md](FORMAT.md). Base/delta compression has
established prior art, including Pekhimenko et al.,
[Base-Delta-Immediate Compression (PACT 2012)](https://www.cs.cmu.edu/~gpekhime/Papers/pact12_pekhimenko.pdf).
The repository makes no claim to have invented those underlying ideas or to
implement that paper's hardware design.

## Identical inputs and storage accounting

All five methods receive exactly the same input bytes for each file. The
comparator divides each file into independent 16 KiB blocks, including the
short final block. This is an experimental block size, not a request to the OS
to compress its virtual-memory pages. No dictionary or history crosses blocks.

Each non-raw method attempts its encoding into a reusable candidate buffer. It
keeps the candidate only when its returned length is nonzero and strictly
smaller than the original block. Otherwise it stores that block unchanged.
The custom codec's mode byte and other internal headers are included in this
decision. An Apple encoder returning zero triggers raw fallback; a custom
encoder violating its declared bound fails the run.

For **every** block in each non-raw method, including fallback blocks, the
container charges a 16-byte table entry: an eight-byte payload offset, four-byte
stored length, and four-byte raw/compressed marker. The C code asserts this
entry size. Reported storage is:

```text
non-raw method: sum(stored block bytes) + 16 × ceil(input bytes / 16384)
raw baseline:  input bytes
ratio:         reported storage / input bytes
```

Consequently, an incompressible file can occupy more reported storage with a
codec than with the raw baseline. A ratio below one means fewer representation
bytes. The table is an experimental container shared by the four codec methods;
it is not Apple's internal memory-management metadata.

This accounting excludes the retained source, reconstruction destination,
candidate buffer, unused allocation capacity, Apple scratch buffers, allocator
and OS overhead, and global container configuration such as original length.
The program keeps the source, packed storage and output allocated together.
`total_storage_bytes` is therefore a calculated representation size, **not
measured RSS or RAM freed**.

## Timing protocol

The published runs use a single thread, one untimed warm-up trial per method,
then nine measured rounds. Every round runs all five methods in a deterministic
Fisher-Yates permutation. The JSON records the seed and actual order; the
runner starts with seed `20261030` and increments it for each manifest entry.
The warm-up order is fixed, not shuffled.

Each trial times an encode batch followed by a decode batch using
`CLOCK_MONOTONIC`. Both batches reuse the same buffers. To reduce timer noise
for short inputs, the fixed number of iterations is:

```text
min(1024, max(1, ceil(8 MiB / input bytes)))
```

The batch duration divided by that count is one per-buffer sample. Larger
inputs use one iteration. Every reported time is the median of nine samples;
all samples remain available in JSON, with minima and maxima also in the CSV.
There is no cache flush, CPU affinity control, frequency lock, or isolated
thermal environment. These are warm-buffer timings; warm does not mean the
entire input necessarily fits in a CPU cache.

Timed encoding includes page inspection, codec selection, raw fallback,
candidate-to-packed copies, and page-table writes. Timed decoding includes
table traversal, bounds checks, raw copies, and reconstruction of every byte.
Allocation, file I/O, corpus conversion and validation are outside the timed
region. Scratch buffers are sized through Apple's API and allocated before
the warm-up.

After **each batch pair**, the output must match the entire source with `memcmp`
and an FNV-1a 64-bit checksum. Validation is not run between inner iterations.
The comparator also requires representation sizes and raw/compressed page
counts to remain identical across trials. A failure aborts the run. The
checksum is a validation aid, not a cryptographic integrity mechanism; pinned
source and dataset provenance use SHA-256 separately.

## Freeze, corpus and reproduction

The runner verifies input sizes and SHA-256 values against the manifest,
compiles with `clang -O3 -Wall -Wextra -Werror ... -lcompression`, and writes
`frozen-config.json` before measuring. That record contains the codec and
comparator source hashes, binary hash, manifest hash, compiler, OS and machine
details, and protocol. It checks the source hashes again after measurement and
refuses to overwrite a nonempty results directory.

V2 was developed using the initial corpus. Its source was then retained for the
new UCI household-power evaluation, rather than retuned after viewing those
results. The initial and new-run frozen records allow comparison of source
digests. Historical V1/V2 results remain evidence of development runs, not a
claim that all their inputs were unseen by V2. Public historical manifests
redact local paths; their records distinguish the original and published
manifest hashes.

The larger evaluation retains all 2,075,259 UCI source rows. Its row-major and
column-major fixed-point files preserve numeric values, timestamps and
missingness; its two text partitions preserve the original text bytes. These
are alternative layouts or partitions of **one source corpus**, not four
independent datasets. Every method receives each layout. Conversion cost is
excluded for all methods and is not credited to the custom codec. The binary
representation is not a byte-for-byte reconstruction of original CSV spelling.
See [DATASETS.md](DATASETS.md) and [DATA_LICENSES.md](../DATA_LICENSES.md).

## Interpreting results

A predeclared strict win against a particular reference requires all three:

```text
custom storage <= reference storage
custom median encode time <= 0.90 × reference median encode time
custom median decode time <= 0.90 × reference median decode time
```

This operational threshold is not a statistical significance test. A better
ratio alone is a storage win; a faster decode alone is a decode win. Neither
establishes an overall strict win. Comparisons must name the reference, input,
layout and machine. Related files must not be counted as independent evidence
of broad generalization.

The experiment does not measure cold random access, page-fault service,
concurrent application performance, cache-controller implementation, power,
energy, DRAM timing, or an OS-wide compression policy. System-wide `vm_stat`
and swap snapshots are context only: other processes contribute to them, and
their changes cannot be attributed to this codec. A claim of beating Apple's
memory subsystem would require a different, substantially broader experiment.
