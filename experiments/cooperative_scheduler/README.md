# Cooperative scheduler with real stack switches

This AArch64 QEMU guest combines the real [`scheduler_core`](../scheduler_core/README.md) Rust library with a copy of the integer stack-switch assembly from [`cooperative_tasks`](../cooperative_tasks/README.md). The scheduler is linked as a path dependency; the switch assembly is copied into this guest and is not dynamically linked to the other demo. No other component is modified.

Two functions run on distinct stacks. Each explicitly reports a yield, block or finish action and switches back to the central driver. The driver applies that action to `scheduler_core`, selects its next ready task, maps the **returned task ID** to one of two statically registered contexts, and switches to that stack. An expected order is checked separately; this is not a generic dynamic N-task dispatcher. The recorded fixture selects `A1, B1, A2, A3, B2`: B blocks after B1, the driver injects a **synthetic wake event** after A2, A finishes, and B resumes and finishes. The guest checks scheduler states, empty queue, disjoint aligned stack ranges, volatile on-stack sentinels and the integer registers `x19`–`x29` plus SP across resumed switches. `x30` is stored as the return address used by the switch. Finished functions remain suspended; they are not cleaned up or independently destroyed.

The scenario uses a single CPU and masks interrupts. There is **no preemption**: a function that does not yield can stop progress. Tasks are functions in the same address space, not isolated processes; their requested actions and identities are trusted. The code has no timer, interrupt-driven wake, GIC, kernel integration, cross-CPU synchronization, security enforcement or integration with the GUI, MCP, persistence or real user programs. The event used to wake B is not delivered by a hardware or OS interrupt.

The switch is only an integer subset of the ARM64 ABI. It does not save FP/SIMD registers, FPCR/FPSR, `x18`, PSTATE/DAIF, exception registers or thread-local state. The soft-float target and disabled NEON do not establish compatibility with arbitrary AAPCS64 programs. Its fixed 16 KiB stacks have no guard pages or overflow prevention; the checks cover only the test points. The test allocator is fixed-size and non-reclaiming. This is not a production-safe scheduler, and no performance comparison with macOS is implied.

## Reproduce

Install Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3. From this directory:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/cooperative-scheduler-demo \
  --output-dir /path/to/output
```

The QEMU TCG guest has no network, disks or host-directory sharing. The host harness checks the image hash before and after execution and records its QEMU version and serial trace; do not rebuild while it runs. See the [synthetic scenario evidence](evidence/cooperative-scheduler-evidence.json). The fixed scenario demonstrates the tested integration, not general fairness, preemption or security.
