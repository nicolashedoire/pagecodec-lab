# Timer preemption of two EL0 loops in QEMU

This single-CPU AArch64 QEMU guest combines a bounded EL1 MMU setup with timer-driven selection of **two actual EL0 assembly loops**. Neither loop yields. The guest gives them distinct 4 KiB `SP_EL0` stacks and a shared EL0 read-only/executable code page, then a virtual timer and QEMU GICv2 PPI 27 preempt them. A fixed, preallocated two-ID ready ring selects the next loop; this is a separate demonstration, not an integration of the general `scheduler_core` API.

The lower-EL AArch64 IRQ vector saves `x0`–`x30`, `SP_EL0`, `ELR_EL1` and `SPSR_EL1` on the EL1 exception stack. It acknowledges the actual GIC interrupt, disables the timer, writes EOI and copies the 272-byte frame to a preallocated frame for that actor. It then requeues the interrupted runnable actor, selects a ready ID, rearms the timer, releases its EL1 IRQ frame, restores the selected user frame and returns with `eret`. No allocator or serial output is used in IRQ context; virtual GIC/timer MMIO and a bounded in-memory journal are necessary. Unexpected vectors or invalid state stop in a nonprinting fatal loop.

The guest checks eight actual timer IRQ records, alternating selected IDs, increasing per-loop counters, each interrupted PC within the relevant EL0 loop, each saved EL0t SPSR and separate `SP_EL0`, and each IRQ's EL1 stack bounds. It checks the final captured integer frames for all `x0`–`x30`, with `x19` used as the loop counter and the others seeded with sentinels. The eighth interrupt stops the timer and branches to an EL1 finish function, so the final ready/running values are a residual snapshot from the last interrupted task, not a usable post-shutdown scheduler state. Journal output and counter values are synthetic evidence, not a benchmark.

The MMU maps the guest image, the two user-stack pages, one UART page and two GIC pages. User code is EL0 read-only/executable; user stacks are EL0 read/write and execute-never. The guest clears both stack pages before EL0 and checks selected `AT S1E0R/W` translations reject access to kernel data, UART and GIC. It does **not** attempt a real EL0 fault or SVC in this image: those paths are fatal here. The separate [`user_mode`](../user_mode/README.md) experiment demonstrates one controlled EL0 fault and SVC, but it is not integrated into this image.

These two stacks remain mutually accessible inside the **same address space**. The loops do not actually access their stacks; their separate SP values and permissions are what this scenario verifies. There is no general process isolation, ASID or address-space switch, task creation, blocking, wakeup, idle path, nested interrupts, SMP, user syscall dispatcher, guard-page recovery or DMA isolation. FP/SIMD, FPCR/FPSR and other architectural extension state are not saved or tested. The GIC and physical addresses are QEMU-specific, not Apple hardware support. No real-time or performance claim is made.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/el0-preemption-demo \
  --output-dir /path/to/output
```

The host launches one QEMU TCG Cortex-A57 VM with GICv2, no network, no disks and no host-directory sharing. The harness checks guest hash before and after execution and records QEMU version and synthetic serial output; do not rebuild the image during the run. See the [recorded VM evidence](evidence/el0-preemption-evidence.json). It supports this fixed test, not general correctness or security.
