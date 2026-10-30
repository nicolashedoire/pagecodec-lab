# macOS scheduler: source-grounded research

This investigation is separate from the compaction benchmarks. It has **not**
replaced or improved the macOS scheduler. It identifies a specific decision
worth testing and the reference behavior a fair comparison must preserve.

## Machine and source identity

On the research machine, read-only system queries reported scheduler `edge`,
12 CPU cores, including 8 Performance and 4 Efficiency cores. The running
kernel identifies itself as Darwin 24.1.0 / `xnu-11215.41.3~13`, ARM64e. The
[exact public XNU source tag](https://github.com/apple-oss-distributions/xnu/tree/xnu-11215.41.3)
is available. The scheduler name is a live system observation; the following
policy details are from that matching **public source**, not a completed
function-by-function decompilation of the running ARM binary.

XNU combines **Edge**, which selects processors and clusters, with **Clutch**,
which organizes runnable threads into groups and quality-of-service buckets.
See Apple's [scheduler dispatch table](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L3731-L3764)
and [group structures](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.h#L209-L219).
Preferences can be supplied and changed by a separate performance controller;
they cannot be reduced to a universal mapping from QoS to a core type.
[Source: thread-group preferences](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L1442-L1478),
[matrix updates](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L3824-L3866).

Apple already uses a lightweight statistical estimate based on average load and
execution duration. An alternative must be compared with this reference, not
with an imaginary scheduler that uses no prediction at all. See the
[load estimate](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/processor.h#L670-L697)
and [execution-time updates](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_prim.c#L9553-L9589).

## Candidate decision: when to migrate

In the public source, `sched_edge_migration_check` examines allowed alternative
clusters. An idle alternative can be selected immediately; otherwise the load
difference must exceed a migration weight. The search considers same-type
clusters first, subject to other constraints. [Migration check](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L4510-L4567),
[search order](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L4621-L4680).

**Hypothesis, not a result:** an alternative estimate could consider expected
waiting time, remaining work and cache/migration cost, and move a thread only
when the estimated benefit exceeds both cost and uncertainty. Short jobs,
bursts, memory contention and mixed workloads are important counterexamples.
Any design must retain the existing priority, affinity, shared-resource and
fairness rules. Measured tail latency, throughput, migration count, fairness and
the predictor's CPU cost would all matter.

## A grounded route to testing

Apple publishes a [Clutch test harness](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/tests/sched/sched_test_harness/sched_clutch_harness.c#L10-L55)
that includes the actual scheduler source and substitutes test state and time.
However, its published [configuration](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/tests/sched/sched_test_harness/Makefile#L6-L23)
models one CPU and one cluster and requires private test dependencies. It is
not an immediately usable multicore Edge benchmark.

The intended sequence is to reproduce Apple's decisions for identical states,
then change only a specified estimate and compare on frozen adversarial
scenarios. Any subsequent experiment using our own real threads would still run
under the host's scheduler; it would not itself replace or prove an improvement
to Edge. The performance controller's live parameters and unobserved private
components are additional limits on claiming equivalence to the whole system.

## Functional extraction on the research Mac

A separate local experiment extracted 12 source blocks byte-for-byte from the
matching public XNU tag, including nine decision or support functions, and
compiled them with explicit simulated cluster-state adapters. Both the normal
and ASan/UBSan builds passed 323 functional assertions. This tests only the
extracted subset under synthetic conditions; it is not Apple's complete test
harness, does not migrate real macOS threads and contains no timing comparison.
The synthetic topology is a fixture, not a claim about the host's live cluster
layout. The Apple source and generated extracts remain outside this repository.
The [reproducible fixture](../experiments/scheduler_edge/README.md) downloads
and verifies the matching public source before extracting the blocks locally.

Apple's source itself identifies a potential optimization: account for a
thread's recent history across clusters when choosing a processor. See the
[upstream source comment](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/kern/sched_clutch.c#L4889-L4907).
This is a concrete direction to examine, not a claim that the idea is novel or
that a performance problem has already been proven.
