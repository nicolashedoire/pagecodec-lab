# Preemptive blocking, synthetic wake and idle in QEMU

This single-CPU AArch64 QEMU guest extends [`preemptive_scheduler`](../preemptive_scheduler/README.md). Two fixed CPU-bound assembly loops run on separate stacks without voluntarily yielding. A virtual-timer IRQ saves their integer exception frames and uses a preallocated two-entry FIFO ring with `ready`, `running` and `blocked` task states. It chooses the next context by ID and resumes it with `eret`. The code is a fixed demonstration, not a generic scheduler or process system.

The test deliberately scripts these actions in the timer handler:

| Timer IRQ | Interrupted context | Synthetic scheduler action | Next context |
| --- | --- | --- | --- |
| 1 | A | Block A | B |
| 2 | B | Block B; ready queue becomes empty | Idle |
| 3 | Idle | Wake A | A |
| 4 | A | Wake B and requeue A | B |
| 5–7 | B, A, B | Requeue the interrupted runnable task | A, B, A |
| 8 | A | Stop the experiment | Finish |

The idle context has its own stack and an assembly loop containing `wfi`. The third IRQ is verified to interrupt an address inside that loop; this does **not** independently prove that `wfi` had already executed or blocked before the IRQ. The wake actions are deterministic **synthetic events keyed to the timer tick**, not an application waiting on I/O or an external event. Enqueueing rejects invalid IDs, queue overflow and a duplicate already queued ID; waking requires a blocked state. The guest checks actual selected IDs, recorded state pairs and ready counts at each step, task counter progress, task/idle PCs and SP/SPSR, and the final integer frames for both tasks and idle.

In each IRQ log, `task` and numeric `next` use 0 for A, 1 for B and 2 for idle; `next=finish` means no task was selected. `states` packs A in the low byte and B in the next byte, with 1 = ready, 2 = running and 3 = blocked; `ready` is the queue length. Action codes are 1 = block A, 2 = block B, 3 = wake A, 4 = wake B, 0 = ordinary preemption/requeue and 5 = finish. These codes describe this fixture only.

The IRQ handler performs no heap allocation and emits no serial output. It necessarily accesses virtual GIC and timer registers and writes a bounded journal in guest memory. Unexpected conditions stop in a nonprinting fatal loop; the harness times out. After the eighth IRQ the timer remains disabled and a separate finish function checks and prints the journal outside IRQ context. The terminal queue/state values are a residual snapshot from the last interrupted task, not a reusable scheduler state after finish. The [recorded synthetic trace](evidence/preemptive-blocking-evidence.json) contains actual observations; counter values are not benchmarks.

This experiment does not implement a generic task blocking API, event source, arbitrary task creation, dynamic queue capacity, completion or cleanup. All functions share one address space; it provides no process isolation, authentication, real-time guarantee or native Apple support. It does not handle nested/unexpected IRQs, SMP, user mode, address-space changes, guard pages or stack overflow. FP/SIMD, FPCR/FPSR and other extension state are not saved. It is not integrated with `scheduler_core`, application I/O, MCP, the graphical guest or macOS, and makes no performance claim.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/preemptive-blocking-demo \
  --output-dir /path/to/output
```

The QEMU TCG guest uses GICv2 and has no network, disks or shared host directories. The harness verifies the eight serial records, hashes the image before and after the run and records the QEMU version. Do not rebuild the image during the run. This is evidence for the one scripted guest scenario, not general scheduler correctness.
