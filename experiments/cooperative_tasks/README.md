# Cooperative ARM64 stack switching

This QEMU-only `no_std` Rust guest executes **two real functions on separate stacks** and switches between them voluntarily. It is an early context-switch experiment, not an operating-system scheduler or multiple processes. A single CPU starts with interrupts masked; all functions share one address space and can access the same memory.

The ARM64 assembly switch saves and restores the integer AAPCS64 callee-saved registers `x19`–`x29`, the stack pointer and `x30` as the return address used to resume execution. The first entry to each task uses an initial `x30` pointing to its function, and `ret` transfers control to it. The task functions explicitly yield in the order `main → A → B → A → B → main`; they are then left suspended. The test seeds `x19`–`x29` and verifies them plus SP on resumed calls in A, B and main. It checks the main, A and B stack ranges are disjoint and 16-byte aligned, and verifies volatile sentinels actually located on A and B's stacks survive suspension. An empty descending stack may have SP exactly at its upper boundary; the VM scenario explicitly checks that boundary and rejects the lower boundary and an address above the stack. Linker and static addresses used for range comparisons are observed through an opaque instruction to avoid relying on compiler assumptions about adjacent linker symbols. If a range check fails, guest-only hexadecimal stack values are printed for diagnosis.

## Limits of this experiment

This switch implements only the stated **integer subset**, not a complete platform ABI or exception context. It does not save or restore FP/SIMD registers (including callee-saved portions of `v8`–`v15`), FPCR/FPSR, `x18`, PSTATE/DAIF, exception registers or thread-local state. `x18` is shared and must not be treated as task-local. The build targets `aarch64-unknown-none-softfloat` with NEON disabled, but that does not prove compatibility with general AAPCS64 programs. It has no timer, preemption, interrupt-safe switch, nested-exception handling, memory protection, process isolation, synchronization or integration with `scheduler_core`.

The two 16 KiB stacks are fixed static regions with **no guard pages or overflow detection**. Range checks only validate the test points; they cannot prevent an overflow. Each task must yield explicitly, and one that never yields will retain control. There are no interrupts enabled in this demonstration. Nothing here modifies or runs on the host kernel or physical hardware.

## Reproduce

Install Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3. From this directory:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/cooperative-tasks-demo \
  --output-dir /path/to/output
```

The guest runs in an isolated one-CPU QEMU TCG VM without network, disks or host directory sharing. The host harness records the guest hash, QEMU version and synthetic serial log; do not rebuild the guest while it runs. The [recorded output](evidence/cooperative-tasks-evidence.json) proves the tested voluntary sequence and checks in that guest, not arbitrary task programs, ABI completeness or production safety.
