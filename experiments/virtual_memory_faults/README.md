# Controlled EL1 write-permission fault

This standalone AArch64 `no_std` guest extends the bounded QEMU MMU setup from [`virtual_memory`](../virtual_memory/README.md) with a controlled exception path. It does **not** modify the original experiment. Its 4 KiB, four-level identity mapping covers the guest image, guest stack and one QEMU UART page; text is read-only/executable, and selected data and device pages are execute-never. The VM uses one CPU with interrupts masked, no network, no disks and no shared host directories.

The test page is a page-aligned Rust **`static mut`** in a dedicated `.roprobe` linker section. The guest checks its placement and maps that page read-only for EL1 in the normal build. This avoids relying on writing to a Rust immutable object if the protection were accidentally missing. A small assembly function attempts one `str` to that page. The EL1 SPx synchronous exception handler checks all of the following before changing the return address:

- `ESR_EL1` exception class `0x25` (data abort at the current exception level), instruction length, write bit, valid FAR, no stage-1 table-walk flag, and DFSC `0x0f` (level-3 permission fault).
- `FAR_EL1` equals the synthetic page's targeted address, and `ELR_EL1` equals the exact controlled store instruction.
- Exactly one fault is recorded; `ELR_EL1` is advanced by four bytes, so `eret` resumes after that one instruction. The handler saves/restores integer registers, while SPSR remains in its system register; this test does **not** separately seed and verify every register or NZCV.

The guest reads all 512 64-bit words of the page before and after the fault and checks they remain unchanged. It also performs selected translation queries inherited from the MMU demo. The experiment does not retry the instruction or implement general fault handling: any unexpected vector, syndrome, address or PC takes a fatal path.

## Separate writable control

The `writable-control` build maps only the synthetic probe page writable. It executes the **same store instruction**, verifies that no fault occurred and the target word changed, and checks the other 511 words stayed unchanged. The probe remains a mutable Rust object in both builds. This is a separate guest image and is not evidence of isolation or protection for real user data.

## Reproduce both images

Install Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3. From this directory, use separate build directories so one image does not overwrite the other:

```sh
CARGO_TARGET_DIR=target/fault cargo build --release --locked
CARGO_TARGET_DIR=target/control cargo build --release --locked --features writable-control
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/fault/aarch64-unknown-none-softfloat/release/virtual-memory-faults-demo \
  --output-dir /path/to/output
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/control/aarch64-unknown-none-softfloat/release/virtual-memory-faults-demo \
  --output-dir /path/to/output --writable-control
```

The harness hashes each guest image before and after its QEMU run and records the QEMU version and synthetic serial output. The two recorded logs are [read-only fault](evidence/virtual-memory-faults-evidence.json) and [writable control](evidence/virtual-memory-control-evidence.json). Do not rebuild an image while it runs.

This is one single-CPU QEMU Cortex-A57 EL1 scenario. It does not establish physical Apple Silicon behavior, process or EL0 isolation, a general exception subsystem, FP/SIMD context preservation, nested exception handling, SMP/TLB shootdowns, DMA safety, general fault recovery, preemption, or performance. Interrupts remain masked. The experiment tests one write-permission fault and a matching control, not all page permissions or fault cases.
