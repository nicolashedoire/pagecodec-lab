# Controlled EL0 entry, SVC and protected kernel data

This AArch64 QEMU guest extends the bounded single-address-space MMU setup in [`virtual_memory`](../virtual_memory/README.md). It enters a small synthetic function at **EL0**, receives controlled synchronous exceptions at EL1 and returns to EL0. It does not implement a general process model or change the original experiment.

The linker places user code and a dedicated 4 KiB user stack on separate page-aligned ranges. The guest checks the ranges and installs four-level, 4 KiB identity mappings: kernel code/data and the QEMU UART are inaccessible at EL0; user code is EL0 read-only/executable and privileged-execute-never; the user stack is EL0 read/write and execute-never. The stack is a `NOLOAD` page outside the boot BSS range, so the guest explicitly clears and checks all 4,096 bytes before EL0 entry. Selected `AT S1E0R/W` queries check these permissions, and descriptor bits are inspected; this is not an exhaustive page-table audit.

The guest installs a 2,048-byte-aligned EL1 vector table, initializes `SP_EL0`, `ELR_EL1` and an EL0t `SPSR_EL1` with interrupts masked, then executes `eret`. The user code writes a stack sentinel and attempts a **real read** from a synthetic kernel-data address. The lower-EL AArch64 handler validates data-abort class `0x24`, level-3 permission fault `0x0f`, read direction, `FAR_EL1` and the exact `ELR_EL1`; it advances ELR past that one controlled load. The user code verifies its destination-register sentinel survived.

Next the user code invokes `svc #0x1234`. The handler validates the SVC syndrome, argument registers, EL0 stack and return PC, changes saved `x0` to `0x99`, and uses `eret` to return to the instruction after SVC. The EL0 function verifies that response, a second register and its stack sentinel, then invokes `svc #0x5678`; this final call enters an EL1 finish function and does not return to EL0. Finish checks the recorded exception counts, syndrome and addresses, the EL0t saved mode, the stack sentinel and unchanged kernel data. The handler saves/restores `x0`–`x30` on the EL1 stack for returning paths, but the scenario verifies only the stated sentinels and values—not every register or NZCV.

The address of the synthetic kernel variable is deliberately supplied to the user test; its value is not a secret. This demonstrates denial for that selected address in one guest. It is **not proof of complete process isolation**: there is one fixed address space, no process creation, ASIDs, general system-call dispatcher, scheduling, arbitrary user programs, concurrent tasks, nested-exception recovery, FP/SIMD state management, DMA isolation or Apple hardware support. Unexpected exceptions are fatal; interrupts remain masked. The EL0 stack has no guard-page fault recovery. No performance claim is made.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/user-mode-demo \
  --output-dir /path/to/output
```

The host compiles and launches a one-CPU QEMU TCG guest without network, disks or shared host directories. All mappings, exceptions and checks occur inside the VM. The harness checks the image hash before and after execution and records QEMU version and synthetic serial output; do not rebuild the image during the run. See the [recorded evidence](evidence/user-mode-evidence.json). Relevant background: Arm's [exception model](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/Exception%20model.pdf) and [memory-management guide](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/LearnTheArchitecture-MemoryManagement-101811_0100_00_en.pdf).
