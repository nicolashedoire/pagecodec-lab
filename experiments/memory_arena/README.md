# Guest static frame arena

This Rust `no_std` QEMU guest exercises [`memory_core`](../memory_core/) against an actual, guest-owned static byte array. The array is explicitly aligned to 4 KiB and contains six 4 KiB frames. The allocator tracks those addresses, and one frame within the array is reserved before allocation. The guest allocates the other five, writes a different deterministic byte pattern into every allocated frame, reads each back, checks the reserved frame's pattern, verifies exhaustion and rejects out-of-bounds or misaligned access in the arena helper. It also verifies that freeing the reserved frame, an out-of-range frame and the same frame twice are rejected. It frees one frame, allocates it again, overwrites it with a new pattern, then verifies all other patterns remain intact.

All reads and writes are actual volatile accesses inside the emulated guest. They do **not** read or write native host physical memory. Accesses use offsets checked against the static array and derive the raw pointer from that array's original pointer, rather than converting an integer physical address back to a pointer. No references to the mutable static are created. The demo runs with one virtual CPU and masked interrupts, so no concurrent access is modeled.

This array is reserved inside the guest kernel image. The experiment does not discover usable system RAM, free kernel memory into a general allocator, configure an MMU, establish physical/virtual mappings or protection, manage an application heap, prove DMA correctness, or support Apple hardware. It assumes QEMU `virt` boots with the MMU and caches disabled; it halts if those assumptions are not met. The underlying frame allocator does not clear data on free or enforce ownership, so reuse here must not be interpreted as process isolation or confidentiality.

## Reproduce in QEMU

With Rust, target `aarch64-unknown-none-softfloat`, Python 3 and QEMU installed:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/memory-arena-demo \
  --output-dir /path/to/output
```

The Python host harness launches a single QEMU guest with no networking, disks or shared host directories and collects serial output. Each allocation, read verification, write, free and reallocation emits a synthetic `IO` record with the relative frame index (or `none`), deterministic pattern seed (or `none`) and outcome. The log contains no frame addresses or per-byte dump. The runner verifies key successful and rejected records. The [recorded evidence](evidence/memory-arena-evidence.json) contains the serial log, guest binary hash and QEMU version. The tests run in the virtual machine, not on the host OS.
