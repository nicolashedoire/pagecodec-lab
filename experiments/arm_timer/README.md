# Controlled virtual-timer IRQ in QEMU

This standalone AArch64 `no_std` guest configures QEMU's ARM `virt` machine with **GICv2**, arms the EL1 virtual timer and handles one actual timer interrupt delivered to the virtual CPU. It is a VM-only experiment: no Mac physical interrupt controller, timer, memory or devices are accessed.

The harness explicitly selects `virt,virtualization=off,secure=off,gic-version=2`, a single Cortex-A57 virtual CPU and QEMU TCG. For this configuration only, the guest uses the virtual GIC distributor at `0x08000000`, CPU interface at `0x08010000`, and virtual-timer PPI INTID 27. A real OS must discover platform devices from a reliable hardware description; these constants are not native Apple Silicon addresses or a portable platform interface. Arm's [generic timer guide](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/Generic%20Timer.pdf) describes the virtual timer, the commonly assigned PPI and the need to clear the timer source before GIC deactivation. QEMU's [`virt` documentation](https://www.qemu.org/docs/master/system/arm/virt.html) describes this virtual board and GIC configuration.

The guest checks that it is running at EL1 with `SP_EL1`, a 16-byte-aligned stack and a 2,048-byte-aligned vector base read back from `VBAR_EL1`; it requires the MMU and data cache to be disabled. Its sixteen 128-byte vector slots route only the current-EL-with-SP_EL1 IRQ slot (`0x280`) to a recoverable handler. It configures the virtual PPI and GIC priority/interface, sets a timer deadline based on `CNTFRQ_EL0`, and unmasks IRQ for a deliberately tight loop at a known guest address. No scheduling or user task is involved.

The IRQ handler saves x0–x30 on an aligned stack, reads the GIC interrupt acknowledge register and requires INTID 27, disables the virtual timer source, then writes the acknowledged value to the GIC end-of-interrupt register. It checks the interrupted PC belongs to the known test loop and **deliberately redirects** the exception return to the test exit label. It restores integer registers and executes `ERET`. After return the probe checks x0–x30, the stack pointer and NZCV; the guest checks exactly one recorded IRQ, the acknowledged ID, expected interrupted and resume labels, the timer disabled bit and that the GIC active bit is clear at the time of the check. Unexpected vectors, source IDs or PCs are fatal.

Redirecting the return PC is specific to this probe; it is not a general transparent interrupt return or a context switch. The experiment does not implement timer periodicity, preempt a task, manage a scheduler, support nested interrupts, save FP/SIMD state, protect processes or validate arbitrary PSTATE. It handles only one source and one CPU; interrupts are masked again immediately after the test. It does not demonstrate that a second interrupt could never arrive, only that the source was disabled and the GIC active bit was clear when checked. The test does not support real Apple hardware. The scope of the pass is the single configuration and register probe described here.

The [recorded evidence](evidence/arm-timer-evidence.json) contains synthetic serial events, the guest binary hash and QEMU version. The test runner observes completion and checks those exact events; it does not inspect host memory or access host block devices.

## Reproduce in QEMU

With Rust target `aarch64-unknown-none-softfloat`, Python 3 and QEMU AArch64:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/arm-timer-demo \
  --output-dir /path/to/output
```

QEMU has no guest network, disk or host directory sharing. The host builds the ELF, launches QEMU and collects serial output; interrupt delivery and all probe checks execute inside the virtual machine. Do not rebuild the image while the test is running; the runner checks its hash before and after.
