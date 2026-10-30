# Timer-driven selection from a fixed ready queue

This single-CPU AArch64 QEMU guest extends [`preemptive_demo`](../preemptive_demo/README.md): two fixed CPU-bound assembly loops run on separate guest stacks without voluntarily yielding. A virtual timer and QEMU GICv2 interrupt preempt the current loop. The handler saves `x0`–`x30`, SP, `ELR_EL1` and `SPSR_EL1`, transitions the interrupted task from running to ready, appends its ID to a **preallocated two-entry FIFO ring**, selects a ready ID from that ring, and restores the corresponding frame with `eret`. This is its own small assembly scheduler; it does not link the separate `scheduler_core` library.

The IRQ path uses no heap allocation and does not write to the serial console, storage or network. It necessarily reads/writes the virtual GIC and timer registers to acknowledge, end and rearm the interrupt, and writes a bounded in-memory journal. Unexpected conditions stop in a nonprinting fatal loop; the host runner times out. The journal records the **actual ID returned by selection**, not an inferred next actor. At the eighth interrupt the timer remains disabled and no task is selected; `eret` enters a separate finish function, which verifies and prints the journal outside the IRQ handler.

The guest checks eight actual timer interrupts, the selected IDs, both loops' increasing counters and PCs, saved integer register sentinels, per-task SP and SPSR, the final ring contents, and that the timer is disabled and GIC interrupt inactive. The final ring state is a **snapshot of the instant before the test ends**: it retains task 1 as previously running even though the separate finish function has taken over. It is not a reusable scheduler state after shutdown. Counts are observations, not performance measurements.

Before starting either task, the guest calls the same `select_ready` routine with an empty queue and verifies the explicit “none” result without state mutation. This is a boot-time test only. The running IRQ scenario always requeues its interrupted runnable task, so it does **not** exercise an empty queue from an IRQ and does not implement an idle task. The model has inactive, ready and running states; it does not implement blocked tasks, wakeup, task creation or dynamic queue capacity. The two task IDs and frame slots are fixed.

This experiment is not a general scheduler, process system, isolation boundary, fairness guarantee, benchmark or native Apple hardware support. It does not handle nested interrupts, other IRQs, SMP, user mode, address-space changes, guard pages or stack overflows. FP/SIMD, FPCR/FPSR and architectural extension state are not saved; task code is limited to integer assembly. The guest's one-CPU, caches-disabled identity-mapped assumptions do not establish safety on physical hardware. No latency or improvement over macOS is claimed.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/preemptive-scheduler-demo \
  --output-dir /path/to/output
```

The guest runs in QEMU TCG with GICv2 and no network, disks or shared host directories. The harness checks the guest image hash before and after the run, verifies the serial records and records the QEMU version. Do not rebuild the guest while it runs. The [recorded evidence](evidence/preemptive-scheduler-evidence.json) is one synthetic run, not a proof of general scheduling correctness.
