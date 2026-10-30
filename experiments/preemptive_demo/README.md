# Timer-driven task alternation in a QEMU ARM64 guest

This experimental Rust/assembly guest runs two fixed CPU-bound AArch64 loops on separate 16 KiB guest stacks. Neither loop calls yield or a scheduler. A QEMU GICv2 virtual-timer interrupt captures the interrupted task's x0–x30, SP, `ELR_EL1` and `SPSR_EL1` in a frame on that task's stack. The handler acknowledges the expected PPI 27, disables the timer source, writes EOI, records the interrupted task, counter, PC, SP and SPSR in a fixed journal, then alternates to the other task and rearms the timer. It restores the selected frame and returns with `eret`. After eight interrupts it leaves the timer disabled and enters a separate finish stack for verification.

The guest checks eight alternating interrupt records, each task's increasing counter, each interrupted PC within the relevant two-instruction loop, the per-task SP and distinct NZCV/`SPSR_EL1` values. It checks x0–x30 in the final captured frame for each task: x19 is the running counter and the other integer registers have task-specific sentinel values. It also checks each final frame lies on the corresponding separate aligned stack, and the timer and GIC interrupt are no longer active. The journal is printed **after** the interrupt experiment, not from interrupt context. The [QEMU evidence](evidence/preemptive-evidence.json) records actual counts; they are not benchmark results.

This demonstrates timer-driven alternation of two **fixed synthetic loops** under the tested configuration. It is not a general scheduler, a process or security boundary, a fairness guarantee, a latency benchmark, or native Apple hardware support. It does not test arbitrary Rust task code, blocking/wakeup, nested IRQs, SMP, unexpected or spurious interrupts, stack overflow/guards, exception recovery, user mode or address-space changes. FP/SIMD, FPCR/FPSR and architectural extension state are not saved or tested; the task loops use only integer assembly and the guest targets soft-float with NEON disabled. Do not run code requiring other state on this context path. The implementation has no dynamic allocation in the IRQ handler, no task creation or general runnable queue. Its identity-mapped, caches-disabled guest assumptions are not suitable for native devices. An independent review of assembly and safety invariants is required before treating this as more than a research experiment.

## Reproduce in QEMU

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/preemptive-demo \
  --output-dir /path/to/output
```

The runner starts a single-CPU QEMU `virt` guest with GICv2, no networking, no disks and no host directory sharing. It checks the guest's markers and the eight IRQ records and saves the serial trace, binary hash and QEMU version. Do not rebuild the guest while the runner is active; the runner checks its hash before and after execution.
