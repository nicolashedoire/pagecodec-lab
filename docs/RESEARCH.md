# Research directions beyond whole-page compression

Research date: **30 October 2026**. Project: [pagecodec-lab](https://github.com/nicolashedoire/pagecodec-lab).

**Follow-up:** the three application-level experiments proposed here have now
been implemented. See [measured operations results](OPERATIONS.md) for outcomes,
controls and reproducible code. This review is retained as the pre-measurement
reasoning; statements about future experiments below describe that earlier stage.

This is a selective research review and a proposed experiment, not a new benchmark result. We inspected primary paper methods and evaluation sections, including arXiv manuscripts and relevant publisher/author PDFs. Reported paper results have not been reproduced locally. Proposed combinations below are hypotheses; this review does not establish their novelty.

## Decision

The next experiment should measure **exact operations on compressed typed arrays**, with selective access and bounded temporary memory. Keep the current codec frozen as a baseline. Optimizing another complete-buffer decompressor would miss the most promising change: doing less reconstruction and moving fewer bytes per useful operation.

Three distinct opportunities emerge:

1. Store a small model plus exact corrections, and consult only the corrections needed by a query.
2. Retain a computation recipe instead of every derived intermediate, where replay is deterministic and affordable.
3. Adapt the independently decodable block size and hot cache to actual access patterns, charging all metadata and transitions.

Each component has substantial prior art. A credible contribution would be a specific policy or representation that beats strong existing implementations under a clearly stated workload, rather than a new name for compressed memory.

## Primary literature worth using

### 1. NeaTS: learned models with exact residuals and random access

Andrea Guerra, Giorgio Vinciguerra, Antonio Boffa and Paolo Ferragina, *Learned Compression of Nonlinear Time Series With Random Access*, arXiv 2024 / ICDE 2025. [arXiv:2412.16266v1](https://arxiv.org/abs/2412.16266v1).

Piecewise functions approximate chronological integer values; packed residuals recover every value exactly. Discarding residuals is a separate, lossy mode. The paper evaluates 16 real time series and reports the best lossless ratio among its specialized comparators on 14, and best overall on four. It explicitly measures point access and range retrieval. Construction cost, model parameters, partition lookup and irregular/noisy data matter. This is unusually relevant to our exact fixed-point UCI representation. We should compare with it before claiming a new model-plus-corrections compressor.

### 2. LeCo: lightweight prediction instead of a large neural model

Yihao Liu, Xinyu Zeng and Huanchen Zhang, *LeCo: Lightweight Compression via Learning Serial Correlations*, 2023. [arXiv:2306.15374v1](https://arxiv.org/abs/2306.15374v1).

Small regressors and fixed-width correction arrays permit reconstruction of one value without unpacking the entire partition. Fixed partitions favor access; variable partitions favor size. The inspected version reports up to 3.9× faster Parquet filter scans and 16% higher RocksDB throughput in its integrations. These are conditional system measurements, not universal speedups. Its lightweight models are a better first baseline than a neural predictor with substantial inference and model-storage costs.

### 3. FastLanes and ALP: strong typed baselines

Azim Afroozeh and Peter Boncz, *The FastLanes File Format*, 2025. [Primary paper](https://www.vldb.org/pvldb/vol18/p4629-afroozeh.pdf), [artifact](https://github.com/cwida/fastlanes-vldb2025). No arXiv identifier was verified.

Composable encodings expose 1,024-value vectors and partially decoded representations for useful operations. Its selected PUBLIC_BI corpus occupies 172.05 MB, about 2% less than Parquet+Zstd, but Parquet+Zstd wins the additional TPC-H sample by 15.3%. Its first-row retrieval experiment is not a uniform-random lookup benchmark. Treat encoding cost and workload selection explicitly.

Afroozeh, Leonardo Kuffó and Boncz's *ALP: Adaptive Lossless floating-Point Compression* is a relevant separate float64 baseline: decimal-aware integer conversion plus exceptions, with another mode for high-precision values. [Primary paper](https://ir.cwi.nl/pub/33334/33334.pdf), [code](https://github.com/cwida/ALP). Table 4 reports 21.7 bits/value across 30 datasets versus Zstd's 20.6. Do not credit an int32 conversion against a float64 competitor receiving different bytes; do not equate its L1-resident microbenchmarks with DRAM performance.

### 4. Direct predicate evaluation over bit planes

Arunkumar Mathiyazhagan, *Direct-Operable SIMD Bit-Slicing: A Framework for Memory-Efficient Predicate Evaluation*, August 2026. [arXiv:2608.26368v1](https://arxiv.org/abs/2608.26368v1).

Bit-plane layouts allow comparisons to produce bitmaps without reconstructing all integers. The reported 3.4–10.7× predicate gains are against scalar Java `int[]` scans on the inspected experiments, not an optimized native SIMD baseline. Narrow values and repeated scans are favorable; wide values, updates and isolated reads can erase the advantage. This is a useful experiment template, not sufficient evidence that our C implementation will win. The underlying principle already appears in Li and Patel's [BitWeaving, 2013](https://pages.cs.wisc.edu/~jignesh/publ/BitWeaving.pdf).

### 5. IBEX: smaller independently accessible blocks

*IBEX: Internal Bandwidth-Efficient Compression Architecture for Scalable CXL Memory Expansion*, 2026. [arXiv:2603.26131v1](https://arxiv.org/abs/2603.26131v1).

The design separates 1 KiB compression blocks from 4 KiB OS pages, shares metadata, and can retain an unchanged compressed shadow after promotion. Reported 1.28×/1.40× improvements compare with other compression architectures in SST simulation. They are not measured M4 or ordinary-DRAM speedups. The transferable hypothesis is to reduce read amplification while counting the cost of indices, shadows and a bounded raw cache.

### 6. CRAM: local access has long-standing theoretical and software precedent

*CRAM: Compressed Random Access Memory*, 2010, revised 2012. [arXiv:1011.1708v2](https://arxiv.org/abs/1011.1708v2).

Compressed dynamic strings support local reads and replacement through block structures and code tables. The work combines word-RAM theory with software experiments; the prototype uses amortized update bounds. It does not provide free access through arbitrary ordinary pointers. A compressed-array API by itself therefore cannot be claimed as a new invention. This paper is unrelated to a similarly named recent Linux project.

### 7. TierScape: multi-codec memory policy already exists

Sandeep Kumar, Aravinda Prasad and Sreenivas Subramoney, *Taming Server Memory TCO with Multiple Software-Defined Compressed Tiers*, 2024. [arXiv:2404.13886v1](https://arxiv.org/abs/2404.13886v1).

Warm and cold pages move through tiers combining codecs, allocators and media. Its Linux/Xeon/Optane experiments include real applications, while reported economic benefits depend on assumed media prices. This is prior art for adaptive hot/raw and cold/compressed storage. Google had already deployed compressed far memory and configuration autotuning in [Software-Defined Far Memory, ASPLOS 2019](https://research.google/pubs/software-defined-far-memory-in-warehouse-scale-computers/). Our opportunity must go beyond simply selecting a codec from page hotness.

### 8. Dynamic Tensor Rematerialization: trade storage for recomputation

Marisa Kirisame et al., *Dynamic Tensor Rematerialization*, 2020. [arXiv:2006.09616v2](https://arxiv.org/abs/2006.09616v2).

Evicted intermediate tensors are recomputed from their dependencies when required. A PyTorch prototype demonstrates memory savings but also substantial framework overhead and thrashing at tight budgets. Its alias/mutation handling has documented restrictions. The transferable idea is to choose between keeping, compressing and regenerating an immutable derived array. It does not permit deletion of arbitrary measured input data: the inputs or an exact reconstructive description must remain available.

### 9. PlaneZip: jointly plan compression, transfers and recomputation

Zhufan Gu et al., *PlaneZip: Lossless Bit-Plane Compression of Training Activations Offloaded over PCIe*, 18 October 2026. [arXiv:2610.25033v1](https://arxiv.org/abs/2610.25033v1).

A 128-byte block codec and per-tensor compressibility estimates feed a transfer/recomputation planner. The paper reports 1.71× compression for BF16 activations and 1.39× for FP32, using captured data. Its 31–47% reductions in rematerialization overhead are **trace-simulation** results; hardware area and throughput are estimates. Non-fused attention and sampled tensors also qualify the evaluation. This is very recent evidence for the research direction, not a ready-made result for Apple's unified memory, which has a different transfer topology.

### 10. Progressive precision and specialized formats: useful distinctions

Victor A. P. Magri and Peter Lindstrom, *A General Framework for Progressive Data Compression and Retrieval*, 2023. [arXiv:2308.11759v1](https://arxiv.org/abs/2308.11759v1). Successive correction components refine an approximation, potentially to losslessness. Crucially, this implementation still assumes room for an uncompressed reconstruction plus a temporary component. Progressive retrieval alone therefore does not prove lower application RAM usage.

Tianyi Zhang et al., *DFloat11*, 2025, revised January 2026. [arXiv:2504.11651v3](https://arxiv.org/abs/2504.11651v3). Entropy-coded BF16 weights and GPU decompression reduce evaluated model storage by about 30% while preserving weights. Large speedups are compared with CPU offloading under a GPU-memory constraint, not always-resident raw weights. The authors explicitly report nonzero decompression overhead and GPU-only evaluation. This is a strong specialized example, not a general RAM format.

### Hardware boundary

[SIMDRAM, arXiv:2105.12839](https://arxiv.org/abs/2105.12839) moves bit-serial operations into DRAM; its proposed design is evaluated in simulation. [Compression-aware LLM memory controllers, arXiv:2503.18869](https://arxiv.org/abs/2503.18869) combine reversible layouts with a separate quantization component; their headline hardware throughput is modeled. Software on our M4 can test layouts and fused operations, but cannot validate a new DRAM controller or reproduce those hardware claims.

## Local diagnostic: where there may still be structure

We performed one additional **exploratory profile**, not a compression benchmark, on the complete existing UCI numeric corpus:

- Every consecutive calendar-minute timestamp increases by exactly one.
- `Sub_metering_1` is unchanged across approximately **94.98%** of adjacent pairs with both values present.
- `Global_intensity` is unchanged across approximately **63.46%** of such pairs.

The exact timestamp series can be described by an origin, a step and a count, with explicit checks for exceptions in other datasets. Repeated measurements suggest run-based or predictive representations. These observations do not establish a new compression ratio or a speed gain. This corpus is now development data; fresh validation requires independently sourced data and access traces.

## Proposed experiments — our synthesis, not established results

### A. Exact answers from a model and selectively read corrections

For each immutable integer block, store a cheap predictor `p(i)` and an exact correction `r(i)` such that `x(i) = p(i) + r(i)`. Store correction bits in independently addressable groups, with a validity bitmap for missing values and conservative residual bounds.

For a `>= threshold` query, bounds can sometimes decide the answer before reading corrections. If `p(i) + lower >= threshold`, the value certainly passes. If `p(i) + upper < threshold`, it certainly fails. Only ambiguous values need finer correction bits. Keep every residual: the answer remains exact, and ordinary reads can reconstruct the original value. A poorly predicted block can use a conventional codec or raw form.

**Candidate research question:** can a cost model choose among certificate-only evaluation, direct bit-plane evaluation and ordinary vector unpacking, using query selectivity and exception density, while keeping only one authoritative value representation? The proposed contribution is this precise choice and its measured benefit; prediction, bounds, bit slicing and model-based compression are established techniques.

**Failure cases:** thresholds near most values; broad residual intervals; effectively random low bits; high update rates; model lookup and branching more expensive than an optimized raw SIMD scan. An extra index or cached summary can silently consume the memory being saved, so count it.

### B. A memory object can hold a recipe

For an immutable derived array, retain its inputs and a deterministic operation graph instead of always retaining its full output. Choose among raw output, compressed output, and recomputation. Demand can materialize only a requested tile. This can also avoid creating a large structured matrix in the first place.

**Candidate research question:** can a bounded scheduler share compressed checkpoints between multiple downstream queries, choosing retention by saved byte-seconds versus reconstruction CPU cost? Start with exact integer feature pipelines. Compare against ordinary caching, compression-only policies and recomputation-only policies.

**Failure cases:** unpredictable reuse, expensive source operations, mutable dependencies, randomness, side effects and nondeterministic floating-point reductions. Replay cost includes retained inputs and dependency metadata. Bit-identical floating-point replay requires additional restrictions; mathematical equality is insufficient.

### C. Make access granularity a measured choice

Test independently decodable logical blocks of 256 B, 1 KiB, 4 KiB and 16 KiB. A small query should pay for its requested block, not necessarily for a whole 16 KiB page. Compare packed blocks with offsets against fixed slots plus exceptions. Add a bounded raw cache only after measuring the uncached variants.

**Candidate research question:** can a small set of access classes justify changing granularity without migration and metadata costs consuming the benefit? Include abrupt access-pattern changes and writes; count recompression and temporary duplicates. This is an application-level experiment, not transparent kernel memory or a physical-page-size change.

## The first experiment to build

Prioritize **A**, initially with fixed-size blocks and a simple frame-of-reference. Do not combine three new mechanisms in the first comparison. A concrete query is `COUNT(*), SUM(Global_reactive_power) WHERE Global_active_power >= threshold`, restricted to variable row windows and valid measurements. The changing window prevents one cached global aggregate from trivializing the task.

1. Support exact `count_ge`, filtered integer `sum`, `get` and `read_range`. Specify missing-value handling and overflow behavior. Preserve complete input bits for ordinary reads.
2. Baselines: optimized raw native SIMD, FOR/FastLanes, a static bit-sliced implementation, relevant LeCo/NeaTS access, and block stores using LZ4_RAW, LZFSE and frozen V2. Compare identical input semantics and include an identical-summary baseline to isolate indexing benefits from compression benefits.
3. Measure construction separately and amortized over 1, 10 and 100 queries. Include uniformly random and clustered reads, full scans, selective filters, changing thresholds and a small-write workload. Label generated query traces as synthetic.
4. Count payload, models, validity masks, offsets, summaries, scratch, cached raw blocks and allocator slack. Release the original source after construction for resident-store tests. Report logical allocation, process footprint and peak temporary footprint separately; macOS compression can affect RSS.
5. Use UCI only for development, then freeze choices and test at least two independent real corpora. Do not inflate data size by repetition. Run paired repeated experiments in separate processes and report uncertainty across independent runs.
6. **Proposed go/no-go gate, not a measured result:** at least 25% lower resident representation size than raw and at least 10% lower end-to-end query time than the best relevant compressed baseline, on two independent read-heavy workloads. Also report p99 and the worst regressions; success on selected operations does not imply universal improvement. Reject a claimed adaptive benefit if it disappears against the strongest fixed policy or after charging construction and summaries.

## Mathematical checks worth proving before optimizing

- `decode(encode(x)) = x`, including missing markers and final partial blocks.
- Bounds used to skip correction reads contain the original value for every permitted input.
- Direct predicate/aggregate results equal the explicitly defined raw operation, with overflow excluded or handled identically.
- State transitions preserve exactly one authoritative value and respect a total live-allocation budget, including transitional copies.
- A fallback has a stated maximum expansion including metadata.

These are useful proof targets. They do not certify speed; performance still needs measurements. Existing tests establish selected cases, not these universal theorems.

For the first integer prototype, a short proof obligation makes the design precise. Suppose a block stores `y_i = b + sum_j(2^j * B_ij)` with nonnegative residual bits. For an exact selection bitmap `M`:

```text
sum_{i selected by M}(y_i)
  = b * popcount(M) + sum_j(2^j * popcount(M AND B_j)).
```

This follows by exchanging two finite sums; it is established bit-sliced arithmetic, not a new theorem. With residual width `w`, a prefix of `k` leading bits `h_i` gives the exact interval:

```text
L_i = b + (h_i << (w-k))
U_i = min(L_i + 2^(w-k) - 1, b + maximum_residual).
```

`L_i >= threshold` certifies a passing value; `U_i < threshold` certifies a failing value. Otherwise refine. Validity masks must exclude missing measurements, and an adequately wide checked accumulator must prevent overflow. The implementation question is whether this saves enough work to beat ordinary SIMD unpacking, especially when most values remain ambiguous.

## Scope and novelty discipline

An arXiv record is not itself independent validation. We found examples where attractive headline numbers depended on scalar baselines, cache residency, simulation, or first-row retrieval. A recent M4 scan-model preprint ([arXiv:2606.22423v1](https://arxiv.org/abs/2606.22423v1)) also refits a parameter on its nominally held-out machine and contains an arithmetic discrepancy in its illustrative threshold calculation. It can inspire experiments, but should not anchor performance promises.

This project can investigate how a particular application represents and accesses data. None of the reviewed results supplies a universal compressor that shrinks every possible input, or establishes that our prototype improves Apple's complete memory subsystem.

## OpenAI mathematical results

We inspected official announcements and original theorem statements, algorithm descriptions and selected formal artifacts. **No Lean development was compiled or independently kernel-checked in this review.** A theorem stated by the source, a formalization supplied by the source, independent mathematical validation, and a practical implementation are distinct evidence levels. The [official October release](https://openai.com/index/sharing-ai-progress-in-mathematics/) describes this material as research for the community to examine. The repository catalog used for the relevance search is pinned to [commit `4eaee87548250b2682affda7d05d947c05554bae`](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/CONTENTS.md).

### Shortest common superstring: a direct representation connection

OpenAI, *A Polynomial-Time 2-Approximation for Shortest Common Superstring*, September 24, 2026, family 128. [Original theorem](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/A-Polynomial-Time-2-Approximation-for-Shortest-Common-Superstring-September-24-2026/build/sections/01-introduction.tex), [algorithm](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/A-Polynomial-Time-2-Approximation-for-Shortest-Common-Superstring-September-24-2026/build/sections/07-algorithm.tex).

The manuscript states a polynomial-time construction of a string containing every input string contiguously, with length at most twice the shortest such string. This is **not** a factor-two guarantee against optimal source compression. The inspected construction enumerates quadratically many substrings, so practical construction memory can be substantial. A possible separate experiment is an immutable shared string arena with offset/length lookup. Count the index, alignment and construction workspace; compare duplicate interning and compressed blocks. A scalable overlap heuristic does not inherit the theorem's guarantee. This is better suited to text/metadata objects than the current numerical corpus.

### Matrix-free computation: the most concrete numerical lesson

OpenAI's *First Proof?*, problem 10, February 2026. [Submission, pp.65–68](https://cdn.openai.com/pdf/26177a73-3b75-4828-8c91-e8f1cf27aaa0/oai_first_proof.pdf), [official status](https://openai.com/index/first-proof-submissions/).

The proposed solver computes structured matrix-vector products without constructing enormous Kronecker/Khatri–Rao matrices. The stated iterative cost is `O(qr + n²r + nr²)` after setup. This illustrates memory saved by avoiding intermediates. OpenAI describes the attempt as highly likely correct under expert feedback, not as a Lean-certified submission. Matrix-free Kronecker techniques have [Airola–Pahikkala prior art, arXiv:1601.01507](https://arxiv.org/abs/1601.01507); do not call this an OpenAI invention. Solver convergence depends on conditioning, and floating-point tolerance is different from our codec's exact-bit contract. Transfer the operator-based design, not a universal performance claim.

### Time versus space: useful inspiration with restrictive models

OpenAI, *Simulating One-Tape Time in Two-Fifths-Power Space*, September 25, 2026, family 137. [Original model and statement](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/Simulating-One-Tape-Time-in-Two-Fifths-Power-Space-September-25-2026/build/source/sections/introduction.tex).

The manuscript claims a simulator using `O(T^(2/5) polylog(T+2))` work bits for a restricted one-writable-tape machine's control/halting outcome. Runtime is unrestricted, read-only input storage is excluded, and the final tape need not be materialized. This is not a compact random-access representation of a general process. It reinforces the conceptual possibility of recomputing intermediates, but does not improve on practical checkpointing without an implementable cost model and measurements.

### Cache allocation: a real mathematical connection, not a ready policy

OpenAI, *Squared-logarithmic randomized k-server on arbitrary metrics*, September 24, 2026, family 110. [Main statement](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/Squared-logarithmic-randomized-k-server-on-arbitrary-metrics-September-24-2026/build/source/latex/sections/01_introduction.tex), [computational companion](https://github.com/openai/math/blob/4eaee87548250b2682affda7d05d947c05554bae/preprints/Uniform-computation-of-the-squared-logarithmic-k-server-bound-September-24-2026/build/source/sections/01-introduction.tex).

The main claimed competitive movement bound is `O(log² k)` against an offline optimum under its request model; it promises no computational efficiency. The companion permits an enormous startup delay and additive cost. Ordinary paging already has a sharper logarithmic guarantee, while compressed pages have variable sizes and asymmetric transition costs that need not define a metric. A controller for our application cannot inherit this theorem without a valid reduction. Charge movement costs, but use practical LRU/CLOCK and cost-aware policies as the initial baselines.

### ETH: inspect the formal scope before repeating the headline

OpenAI, *The Exponential Time Hypothesis is False*, October 2026. [Announcement](https://openai.com/index/building-scientific-understanding-with-academic-communities/), [manuscript](https://github.com/openai/math/blob/main/preprints/The-Exponential-Time-Hypothesis-is-False-October-25-2026/paper.pdf), [formalization scope](https://github.com/openai/math/blob/main/lean/docs/378.md).

For fixed `k`, the manuscript states exact counting for `k`-SAT in subexponential time with polynomial working space. The inspected Lean statement has an extra `log log n` factor in the exponent compared with the manuscript's strongest bound. The actual solution has proof bodies; we did not run their checker or audit all dependencies. Neither version asserts polynomial-time SAT or supplies a practical RAM compressor. Streaming bounded intermediate summaries is a design lesson, not evidence of codec throughput.

### Coding-theory headlines do not give source compression

The [August 2026 collection](https://openai.com/index/ten-advances-in-mathematics/) and its [original manuscript](https://cdn.openai.com/pdf/ten-proofs-oai.pdf) include improved upper bounds on separated binary codes and arithmetic-circuit lower bounds. The former concerns error-correcting codes; the latter constrains symbolic computation. Neither directly constructs a better lossless representation of arbitrary input memory. These findings are not selected as implementation candidates.

## Recommendation after the mathematics inspection

Proceed first with the **exact progressive filter plus masked aggregate** experiment. Its correctness can be specified and proved locally, and its useful work can be measured on this machine. Keep the **immutable shared-string arena** as a distinct second research branch. Treat matrix-free operators and rematerialization as a later application experiment, rather than importing a highly abstract time-space theorem into the codec's claims.

The research opportunity is a concrete, measured reduction in the information that must be materialized for a useful operation. Whether our particular design improves on existing work remains an open experimental question.
