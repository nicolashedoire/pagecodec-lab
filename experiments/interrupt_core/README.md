# Controlled AArch64 synchronous exception in QEMU

This `no_std` guest installs a minimal AArch64 EL1 vector table, triggers one deliberate `BRK #0x1234` exception, and resumes immediately after that instruction. It runs only as a standalone QEMU `virt` experiment. It does not install anything on the host or use host devices.

On boot the guest masks exceptions, requires EL1 with MMU and data cache disabled, selects `SP_EL1`, initializes BSS and a stack, sets `VBAR_EL1` and reads it back. It checks that the stack is 16-byte aligned and the table is 2,048-byte aligned. The table has sixteen 128-byte slots; only the synchronous current-EL-with-SP_EL1 slot at offset `0x200` has a recoverable handler. Other slots branch to a fatal path. The handler saves x0–x30 on a 16-byte-aligned stack, checks the BRK64 exception class and immediate and that `ELR_EL1` equals the designated instruction, records the syndrome and fault PC, advances `ELR_EL1` by four, restores the registers and executes `ERET`. An unexpected syndrome or PC is fatal rather than skipped.

The in-guest probe seeds each x0–x30 with a different integer value, triggers the exception, then checks all 31 values and the stack pointer. It also sets and checks NZCV across the exception, so the observed return covers those flags. The Rust caller checks a single trap, the syndrome and 32-bit instruction-length bit, exact exception site and `resume = site + 4`. The VM emits only synthetic status messages and the [recorded evidence](evidence/interrupt-core-evidence.json) includes those messages, the guest binary hash and QEMU version.

This is **not** an interrupt controller, timer, IRQ/FIQ handler, scheduler, fault recovery system, process isolation or an MMU. Interrupts remain masked; no device IRQ or nested exception is exercised. It does not save FP/SIMD or other extended state, support exceptions from EL0 or a different stack, validate every PSTATE field, or handle arbitrary synchronous exceptions. Its fatal path is not a general-purpose crash reporter. The guest assumes the QEMU ARM `virt` memory and PL011 serial layout and is not a native Apple Silicon boot implementation. Register preservation is evidenced for the single controlled probe and configuration, not formally proven for every possible machine state.

Arm's [exception model](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/Exception%20model.pdf) describes vector offsets and the architectural roles of `VBAR_ELx`, `ELR_ELx` and `SPSR_ELx`; our implementation is a deliberately narrow experiment, not a complete implementation of that model.

## Reproduce in QEMU

With Rust target `aarch64-unknown-none-softfloat`, Python 3 and QEMU AArch64:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/interrupt-core-demo \
  --output-dir /path/to/output
```

The harness launches one single-CPU QEMU guest with no networking, disk or shared host directories and checks the exact serial outcome. Building and launching are performed by the host; all exception execution and register checks happen inside the VM. Do not rebuild the guest while the test is running; its binary hash is checked before and after the run.
