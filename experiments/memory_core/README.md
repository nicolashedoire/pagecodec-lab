# Physical frame state prototype

`memory-core-sketch` is a small Rust `no_std` **metadata model** for a contiguous physical-frame pool. It uses two fixed-size bitmaps embedded in the allocator; it performs no dynamic allocation for its own metadata. The pool and all reservations are supplied by its caller. The demonstration uses 4 KiB frames and at most 2,048 frames; those values describe this prototype, not the Apple M4 Pro's native page size or memory map.

Construction rejects an unaligned base, empty or oversized pool, address overflow, and reserved ranges that are unaligned, empty or outside the allowed pool. Reservations are half-open, frame-aligned ranges; overlapping reservations are accepted. Allocation returns the lowest available address and skips every reserved frame. Freeing requires an aligned address in the pool that was actually allocated and is not reserved; reserved frames, foreign addresses and double frees are rejected. The entire prototype is single-owner state: the caller must serialize access and ensure different allocator instances do not overlap.

## Scope and important limitations

The allocator does not discover RAM, reserve the kernel, parse a device tree, map pages, manage page tables, initialize an MMU, map or unmap DMA, establish memory protection, zero memory, manage a heap, or track reference counts. It neither establishes that any supplied physical address is real RAM nor makes returned addresses safe to dereference. It has no integration with the graphical guest, boot memory map, or system services. It cannot change reservations after initialization, allocate contiguous runs, or manage discontiguous banks. A linear scan favors clarity over scalability, and there is no allocation-failure recovery beyond returning `None` for an exhausted pool.

Addresses are the only release token: after a frame is freed and reallocated, a stale holder could free the new owner's allocation. The simple double-free check detects freeing an address which is currently unallocated, not this reuse/ownership problem. A real kernel requires authenticated or otherwise protected ownership/lifetime tracking, a trustworthy hardware map, architecture-specific frame/page configuration, concurrency control, failure handling and extensive further verification. No native Mac hardware support is claimed.

## QEMU verification

The AArch64 guest tests a wholly synthetic pool; its addresses are **never dereferenced**. The VM exercises allocation, reservation skipping, exhaustion, valid free and reuse, unaligned/out-of-pool/reserved/double-free rejection, invalid pool/range rejection, overlapping reserved ranges and bitmap boundaries 63, 64 and 2047. It does not test physical memory contents or hardware mappings.

With Rust, target `aarch64-unknown-none-softfloat`, Python 3 and QEMU installed, run:

```sh
cargo build --release --locked --offline --features qemu-demo
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/memory-core-demo \
  --output-dir /path/to/output
```

The guest runs on QEMU `virt` with one CPU, no networking, disks or host directory sharing. The host-side Python harness launches QEMU, records the synthetic serial log and guest image hash, and emits `memory-core-evidence.json`. See the [recorded VM evidence](evidence/memory-core-evidence.json). The logged inputs do not contain personal data, and no native host memory frames are inspected or modified.
