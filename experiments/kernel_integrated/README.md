# One guest: MMU, EL0/SVC and a controlled EL1 timer IRQ

This `no_std` AArch64 QEMU guest combines a bounded MMU configuration, the controlled EL0/SVC scenario from [`user_mode`](../user_mode/README.md), and a single virtual-timer IRQ in **one running image**. It uses one vector table for lower-EL synchronous exceptions and current-EL SPx IRQs. It does not link a general kernel, graphical guest or scheduler, and it does not modify the source experiments.

The one-CPU guest installs four-level 4 KiB identity mappings for its bounded image, a dedicated zeroed user stack and user code page, the QEMU UART page, and only the two GICv2 pages used by this demo. Kernel data, UART and GIC are privileged-only; their selected EL0 read/write translation queries fail. User code is EL0 read-only/executable and the user stack is EL0 read/write and execute-never. The GIC pages use Device-nGnRnE and execute-never descriptors. The guest checks the selected mappings and unmapped neighboring pages; it does not establish an exhaustive memory-policy proof.

The guest enters EL0 with `eret`. Synthetic EL0 code tries to read a kernel-data address; a real lower-EL permission fault is validated using ESR, FAR and the exact load PC, then the handler advances past the controlled load. An SVC request `0x1234` returns response `0x99` to EL0, where the user code checks that result, a register and its dedicated stack sentinel. A final SVC `0x5678` enters an EL1 finish function; it does not return to EL0. Finish checks the recorded fault/SVC state and unchanged kernel data before starting the timer portion.

Only **after** those checks, the EL1 finish function configures the virtual timer and QEMU GICv2 PPI 27, then enters a bounded polling loop with IRQ unmasked. The actual IRQ interrupts that EL1 loop. Its handler saves/restores integer registers, verifies the interrupt ID, disables the timer, issues GIC EOI and returns with `eret`. The resumed code checks the interrupted EL1 PC range, SPSR mode/IRQ bit, stack bounds, one IRQ, timer disabled and no active GIC interrupt. There is no serial output or allocation in the IRQ handler; it necessarily accesses the virtual timer/GIC and records a few words in guest memory. Unexpected IRQ conditions stop in a nonprinting loop and cause the host harness to time out.

**The timer does not preempt EL0 or switch tasks.** The handler saves integer registers, but this test does not independently verify every register or NZCV and does not save FP/SIMD state. It does not provide general process isolation, a generic syscall or interrupt subsystem, nested-exception recovery, task scheduling, graphics, DMA, SMP, durable storage, native Apple support or real-time guarantees. There is one fixed address space; the synthetic kernel address is deliberately provided to EL0 for the denial test. No performance claim is made.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/kernel-integrated-demo \
  --output-dir /path/to/output
```

The host harness starts one QEMU TCG Cortex-A57 guest with GICv2, no network, no disks and no shared host directories. It hashes the image before and after the run and records QEMU version and synthetic serial output; do not rebuild the guest during execution. The [recorded evidence](evidence/kernel-integrated-evidence.json) establishes only this controlled scenario.
