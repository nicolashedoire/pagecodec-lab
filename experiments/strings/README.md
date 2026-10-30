# Immutable string storage experiment

This experiment asks whether shared byte strings save enough storage to justify their construction and lookup costs. It compares four user-space representations through the same object-ID API. It does not change DRAM, integrate with an operating-system memory manager, or establish an advantage over Apple.

The published input views are CMU Pronouncing Dictionary headwords and pronunciations from the same pinned source. They are **two views of one dataset**, not independent corpora. Natural pronunciation duplicates are retained. No suffixes, duplicate objects, or repetitions are manufactured for benchmark measurements. The exact source revision, license and transformations are recorded in [the operations manifest](../../manifests/operations-v1.json). Synthetic fixtures are used only for correctness tests.

## Build and run

Requirements: a C++17 compiler and zlib development headers/library. macOS Command Line Tools provide the compiler and system zlib. On Ubuntu, install the usual compiler toolchain and zlib1g-dev.

From the repository root:

~~~sh
mkdir -p build
c++ -O3 -std=c++17 -Wall -Wextra -Werror \
  experiments/strings/string_bench.cpp -lz -o build/string_bench
python3 tests/test_strings.py --binary build/string_bench
python3 scripts/prepare_operations.py
build/string_bench --input data/operations/cmu-headwords.txt \
  --output build/strings-headwords.json \
  --trials 9 --queries 20000 --seed 20261030 \
  --block-bytes 4096 --candidate-cap 64 --max-overlap-bytes 256
build/string_bench --input data/operations/cmu-pronunciations.txt \
  --output build/strings-pronunciations.json \
  --trials 9 --queries 20000 --seed 20261030 \
  --block-bytes 4096 --candidate-cap 64 --max-overlap-bytes 256
~~~

Without --output, JSON is written to standard output. --only-method concat, intern, bounded_overlap or zlib_blocks selects a single method for isolated process measurements. The default block target is 65536 bytes; the published run explicitly freezes it at 4096 before performance measurements. Changing the block size changes both storage and decompression work.

Input is a binary file with one object per LF-terminated line. Only LF is removed. Spaces, CR, NUL and UTF-8 byte sequences are preserved; no Unicode normalization or tokenization occurs here. An unterminated final line is an object. Empty lines are valid empty objects: a file containing bytes a, LF, LF is two objects (a, empty); a, LF is one; an empty file contains none. The current packed block format rejects an individual object larger than UINT32_MAX bytes; ordinary inputs are not truncated.

## Representations

| Method | Stored payload | Object lookup |
| --- | --- | --- |
| concat | All objects concatenated in input order | Offset and length into the arena |
| intern | Each distinct object stored once | Original object IDs point to the unique copy |
| bounded_overlap | Distinct objects packed using the bounded heuristic below | Original IDs point to exact contiguous slices |
| zlib_blocks | Object-aligned blocks compressed independently with zlib level 1; raw fallback if compression does not shrink a block | Decode the referenced block into reusable scratch, then return the object's slice |

Every method uses a 16-byte object entry (two uint64_t fields), including duplicates and empty objects. The three direct arenas use offset/length. The zlib entry packs a 32-bit block ID and a 32-bit within-block offset into its first field, with a 64-bit object length in the second. Each zlib block also uses a 32-byte descriptor: payload offset, stored length, decoded length and raw-fallback flag. The target block size is soft: an oversized object occupies its own block and is never split. Empty objects need no payload.

The common entry width is an explicit design choice, not a claim that 16 bytes is the smallest possible index. Packed 32-bit entries, variable-width indices, compressed indices, or alternative block directories could change the storage ranking. zlib_blocks is a portable reference implementation, not an optimized compressed cache.

## Exact limits of the overlap heuristic

The implementation first interns exact duplicates. It then builds one arena, choosing among remaining unique objects:

1. Index objects of at least eight bytes by their first eight bytes.
2. Inspect possible arena-suffix lengths from max_overlap_bytes down to eight, capped by current arena length.
3. At each suffix, inspect objects in the matching eight-byte bucket, in reverse insertion order. Stop after candidate_cap candidate inspections total for this append. Inspections of stale or too-short candidates also consume the cap.
4. Select the first exact suffix-prefix match at the longest tested length. Remove already-used candidates lazily. If none is found within the limits, append the first unused object in original order.
5. Append only the unmatched suffix and retain an offset/length entry for each original ID.

With the published settings, at most 64 candidates and 249 suffix-key probes are considered per appended unique object; each byte comparison is at most 256 bytes. Hash-table operations have their usual average-case behavior, not a guaranteed worst-case constant bound. This avoids an all-pairs overlap graph and enumeration of all substrings. It ignores overlaps shorter than eight bytes and longer than 256 bytes, can miss useful candidates after exhausting its cap, and does not search arbitrary interior substrings. A complete match against the arena's current suffix can store an object without appending bytes. It does not truncate objects. It may give no improvement over exact interning.

This heuristic is **not the algorithm in OpenAI's claimed polynomial-time factor-two shortest-common-superstring result** and inherits no factor-two guarantee. That manuscript's bound concerns the length of a superstring containing every input string, not optimal general compression, index bytes or lookup latency. Its actual construction uses all-substring graph machinery. See the pinned [original theorem](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/A-Polynomial-Time-2-Approximation-for-Shortest-Common-Superstring-September-24-2026/build/sections/01-introduction.tex) and [algorithm analysis](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/A-Polynomial-Time-2-Approximation-for-Shortest-Common-Superstring-September-24-2026/build/sections/07-algorithm.tex). Overlap packing and string interning are established ideas; this experiment does not claim their invention or independent verification of that mathematical proof.

## Measurement contract

Each of nine trials shuffles method order. Each method is rebuilt from freshly read input; file reading and parsing are outside construction timing. Construction timing includes hash tables, packing/compression, index creation, final capacity compaction and destruction of builder temporaries. Before lookup timing, every object is checked against the original using its complete length and bytes. The original string vector is then freed.

Every method receives the same two fixed query traces:

- **uniform:** object IDs drawn across the full input.
- **hot:** 90% of selection decisions draw from a seeded random set containing 1% of object IDs (at least one); the remainder draw from the full input. The actual hot-set hit rate can therefore exceed 90%.

The traces describe synthetic access patterns over real objects, not measured application access traces. The seeded generator and emitted method/workload orders support reproducing a run in the same C++ standard-library environment; std::shuffle permutations can differ between library implementations.

Each trace is measured under two contracts, with workload order shuffled:

- **view:** return a borrowed slice valid until the next lookup on that workspace.
- **copy:** copy every returned byte into a preallocated caller buffer before consuming it.

Both contracts consume **every returned byte** in the same non-inlined FNV routine and fold in each object's length. Output checksums must match the original trace for all methods and both contracts. Timings include this consumption cost; they are not bare pointer-lookup costs. The copy destination is passed to that consumer, rather than timing a pointer assignment. Up to 256 warm-up queries run before each measured batch. Caller workspace allocation and verification are outside timing; internal allocation and deallocation performed by zlib's uncompress function remain inside lookup timing.

For zlib, each compressed-block lookup decodes the full block, even after a previous query to that block. The benchmark has no persistent decoded-block cache. Raw fallback blocks can return direct views. The untimed whole-corpus verification pass may reuse its most recently decoded block; that optimization is disabled in timed lookups. A future decoded cache would need its own capacity and policy accounting.

The ns_per_query_samples array contains one **batch-average** value per trial. It is not a sample of individual request latencies and cannot establish p99 request latency. For the empty-input correctness case, the actual query count and reported averages are zero.

## Storage and process memory

- payload_bytes, index_bytes and block_metadata_bytes sum to logical_storage_bytes.
- capacity_storage_bytes counts actual allocated capacities of the payload, object index and block descriptor vectors. This exposes unused vector capacity after shrink_to_fit, which is nonbinding in C++.
- lookup_workspace_view_bytes and lookup_workspace_copy_bytes count actual scratch capacities. resident_*_capacity_bytes adds the appropriate scratch to store capacity.
- The two shared query traces remain allocated and are reported separately as benchmark_trace_capacity_bytes. Original input strings are not retained during timed lookups.
- Small container/control objects, allocator headers, executable/library memory, transient internal zlib allocations and any retained allocator pages are not included in those store-capacity counters. In particular, the decode scratch capacity does not include zlib's own decoder state. Therefore these counters are not a process RSS measurement.
- construction_estimated_peak_bytes is an approximate diagnostic covering source string storage, representation capacities, temporary buffers and estimated hash-table nodes. It can omit allocation slack, allocator headers and zlib internals; short-string storage can be counted twice. It is neither measured RSS nor a proven upper bound.

An isolated /usr/bin/time -l run on macOS can measure whole-process maximum resident size using --only-method NAME --trials 1. That peak includes parsing, oracle validation, construction temporaries and library/runtime memory; it must be labeled **process/construction peak**, not steady-state store occupancy. Releasing the input vectors also does not guarantee that the allocator immediately returns every freed page to the OS.

## Correctness checks

The test script exercises empty files/objects, empty final objects, absent final LF, CR, NUL, UTF-8, exact duplicates, a known overlap, a no-overlap case, oversized compressible objects and raw fallback. It checks all metadata arithmetic, original-order restoration through full verification, matching full-byte query checksums, nine paired trials on a small fixture and all four isolated method modes. The known-overlap fixture demonstrates the implementation path only; it is not benchmark evidence.

For a sanitizer build:

~~~sh
c++ -O1 -g -std=c++17 -Wall -Wextra -Werror \
  -fsanitize=address,undefined -fno-omit-frame-pointer \
  experiments/strings/string_bench.cpp -lz -o build/string_bench_sanitize
python3 tests/test_strings.py --binary build/string_bench_sanitize
~~~

Interpret results as a representation tradeoff for these immutable objects. Include indices, scratch, construction time and all lookup contracts when comparing methods. A payload-only win does not establish a total-memory win, and a store-capacity win does not establish lower application latency or reduced physical RAM usage.
