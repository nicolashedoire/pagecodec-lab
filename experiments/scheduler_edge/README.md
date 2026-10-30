# Apple Edge scheduler: public-source functional extraction

This local study compiles nine unchanged Apple decision/support functions from XNU `xnu-11215.41.3` using the public macOS Command Line Tools. It exercises cluster-selection decisions in synthetic states. It does **not** replace the host scheduler, move real macOS threads between cores, reproduce all of Edge, or measure performance.

From the repository root, retrieve the public sources and run the functional checks:

```sh
cd experiments/scheduler_edge
python3 fetch_upstream.py
python3 build_and_check.py
```

To reproduce without distributing any Apple source files, distribute only `fetch_upstream.py`, `build_and_check.py`, `upstream-manifest.json`, `edge_environment.h`, `edge_cases.c` and this README. Omit `upstream/`, `generated/` and the executables. On a machine with Python 3 and the public Command Line Tools, run:

```sh
python3 fetch_upstream.py
python3 build_and_check.py
```

The first command retrieves the public tag from Apple's GitHub repository without authentication, verifies each Git blob hash before saving, and preserves the upstream source notices. It only needs network access when a verified cached file is absent. `python3 fetch_upstream.py --verify-only` performs a read-only, offline cache check. The bootstrap's offline verification path was executed here; a fresh network bootstrap was not rerun because the exact source files had already been obtained and verified through the GitHub connector.

After the source cache is populated, `build_and_check.py` verifies the Git blob hashes of 13 cached Apple source files, extracts 12 byte-preserved blocks, compiles an ordinary C11 executable and an AddressSanitizer/UndefinedBehaviorSanitizer variant, and runs 323 functional checks in each. The build and checks need no network, login, privileged access, Instruments, private SDK, installation or system modification. All output stays in `generated/` and `evidence/`. The source cache and generated artifacts are excluded by this directory's `.gitignore`. Compilation and functional execution are not timed.

The observed compiler is Apple clang 16.0.0 (`clang-1600.0.26.6`), targeting `arm64-apple-darwin24.1.0`. Both variants passed all 323 checks, without sanitizer diagnostics. This establishes that the selected source functions can execute under the specified adapters; it is not a performance or correctness proof for the full scheduler.

## Native observation versus source execution

Read-only `sysctl` queries on this host returned `kern.sched: edge`, 12 CPUs, eight Performance cores and four Efficiency cores. XNU exposes the scheduler name through [kern.sched](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/bsd/kern/kern_sysctl.c#L5005-L5012). The source tag corresponds to the Darwin build previously identified in the native ARM kernelcache.

This study does not read live runqueues, current edge weights, CLPC state, or application thread state. Its synthetic topology has two P clusters and one E cluster of four CPUs each; this is a fixture, not verification of the host's live processor-set layout. Static source correspondence does not constitute byte-level validation of these scheduler functions in the stripped kernelcache.

## Why the official harness does not build unchanged here

The upstream [tests/Makefile](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/tests/Makefile#L38-L48) includes `$(DEVELOPER_DIR)/AppleInternal/Makefiles/darwintest/Makefile.common` and private framework/header search paths. That internal Makefile is absent from the installed Command Line Tools. Direct compiler probes additionally confirm that `darwintest.h`, `darwintest_utils.h` and `os/atomic_private.h` are unavailable through this public SDK; `stdatomic.h` is available. The SDK also lacks `libdarwintest_utils.dylib`. These are local dependency findings, not a claim that every missing dependency has no public-source replacement.

The official [Clutch harness](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/tests/sched/sched_test_harness/sched_clutch_harness.c#L30-L55) mocks one CPU and one processor set, then includes the actual `sched_clutch.c`. Its [build flags](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/tests/sched/sched_test_harness/Makefile#L6-L23) enable Clutch, not Edge. Its mock `processor_set` has only an identifier and a Clutch root, and its [enqueue/dequeue entry points](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/tests/sched/sched_test_harness/sched_clutch_harness.c#L194-L210) directly use `cpu0`; they bypass the inter-cluster placement path. Merely defining `CONFIG_SCHED_EDGE=1` does not turn this into a multicore Edge harness.

## What executes unchanged

`build_and_check.py` records every source range, URL, byte count and SHA-256 in `evidence/extracted-blocks.json`. Original source notices are retained in the generated files. The extracted functions are:

- `sched_get_pset_load_average` — rounded load average multiplied by mean execution time.
- `sched_edge_thread_shared_rsrc_type` — precedence of the shared-resource policies.
- `sched_edge_pset_available` and `sched_edge_cluster_load_metric` — unavailable-cluster exclusion and load metric.
- `sched_edge_config_set` and `sched_edge_cpu_init_completed` — source initial edge configuration.
- `sched_edge_migration_check` — candidate eligibility and migration threshold.
- `sched_edge_iterate_clusters_ordered` — local-die then remote-die traversal.
- `sched_edge_migrate_edges_evaluate` — homogeneous/heterogeneous search and shared-resource capacity policy.

Additional unchanged blocks provide the edge union, resource type constants and shared-resource policy table. `edge_environment.h` supplies separate, explicitly reduced processor/thread state, C11 atomic loads and bit operations. Interrupt and lock adapters are no-ops because the checks are single-threaded. The adapter structs are not XNU ABI-compatible and cannot be passed to kernel interfaces.

Tests cover initial permissions, the non-idle P-to-E threshold at 63 versus 64 microseconds of estimated delay, idle-candidate handling, unavailable and missing clusters, homogeneous tie-breaking, changes to edge configuration, shared-resource precedence/capacity, fixed-point rounding, all six scheduling buckets, and die-local traversal. The [edge units are documented by Apple](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/kern_types.h#L340-L365). Initial P-to-E weight 64 and disabled E-to-P migration are source initialization values, **not a measurement of current settings**.

The fixture enters only the nonzero-load branch that calls `sched_edge_migrate_edges_evaluate`. It does not exercise the complete `sched_edge_migrate_candidate` wrapper: bound threads, realtime handling, idle preferred clusters, AMP rebalance, availability races and lock switching are outside this extraction. It does not maintain actual runnable queues or execute the EWMA update over time.

## What a fuller Edge harness needs

1. Replace or supply the internal test runner and private atomic helpers while preserving upstream assertions and source boundaries. Public C11 atomics suffice for this single-threaded extraction, not automatically for every XNU atomic primitive.
2. Extend the official shadow states to multiple processor sets and CPUs: topology, online/recommended CPU masks, per-QoS load/running counts, real-time and Clutch queues, per-group preferred clusters, shared-resource flags, thread binding, previous processor and quantum/rebalance state.
3. Initialize Edge and route arrivals through `sched_edge_choose_processor`/`sched_edge_migrate_candidate` before enqueueing. Then model CPU-idle steal, rebalance, preemption and quantum expiry. The current direct `cpu0` enqueue cannot validate these paths.
4. Drive simulated time and workload events, implement the actual load-average updates, and audit queue/lock invariants. Modeling synchronization is separate from proving the code safe under real kernel concurrency.
5. Supply explicit CLPC input traces or configurations. Public XNU exposes [updates to the edge matrix](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L3824-L3866) and per-QoS cluster preferences, but this extraction does not reconstruct the platform controller producing them. A fixed initial matrix cannot be called Apple's complete live policy.

## Candidate research direction

The public source already predicts scheduling delay using load and execution-time averages. A useful next experiment would preserve the Apple decision path and replace one estimate with a per-workload estimate of expected completion time that includes migration/cache-locality cost and uncertainty. Measure held-out workloads, overhead, tail latency, throughput and fairness; do not assume a neural model is better than a simple estimator.

Apple itself marks a possible [per-thread, multi-cluster recency metric](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L4889-L4907) as an optimization opportunity. This is a concrete extension point, not evidence of a novel idea or demonstrated performance defect. No replacement estimator or speed comparison is implemented here yet.

## Evidence files

- `upstream-manifest.json` and `evidence/upstream-verification.json`: exact tag, source URLs, Git blob identity and SHA-256.
- `evidence/extracted-blocks.json`: unchanged source boundaries and hashes.
- `evidence/environment.json`: compiler, SDK, header probes and read-only host scheduler/topology queries.
- `evidence/functional-results.json`: compiler logs, executable hashes, assertion results and sanitizer execution.
- `evidence/local-file-hashes.json`: adapters, fixture, generator and generated-source hashes.

The repository contains only the authored adapters, tests, scripts, documentation and source manifest. Apple sources are fetched locally with their notices preserved; they are not bundled in the repository. No proprietary binaries or Ghidra artifacts are included, and nothing is installed on the system.
