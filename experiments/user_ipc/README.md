# Bounded synthetic EL0 message exchange

This small Rust `no_std` guest demonstrates a **fixed sequence** of calls from two AArch64 EL0 contexts through an EL1 `SVC` handler in single-CPU QEMU. It builds on the memory layout and page-table setup in [address_spaces](../address_spaces/README.md). The two TTBR0 roots map virtual address `0x50000000` to two distinct zeroed 4 KiB physical pages, while kernel mappings are privileged. The roots and per-actor pages are fixed for this experiment. Selected EL0 permissions are inspected with `AT`; this test does not exercise an actual forbidden-address exception.

Each context has its own integer frame and virtual stack address. The handler checks the SVC class and instruction width, saved EL0 mode and masks, expected post-SVC instruction address, active TTBR and expected actor/phase before executing a service. In this fixed eight-call script:

1. A sends an address near the end of its page with a length crossing the page boundary. The handler rejects it before accessing that pointer.
2. A sends `u64::MAX - 1` with length 5. Checked addition overflows and the handler rejects it before access.
3. A sends length 17, above the fixed 16-byte maximum. The handler rejects it before access.
4. A writes `HELLO` to its page and sends it. The handler copies five bytes to a fixed EL1 mailbox and cooperatively switches to B's root and frame.
5. B observes its own page initially has zero in the corresponding location, receives the five mailbox bytes into its own page, and checks the contents.
6. B sends `OK`; the handler copies the reply to a fixed EL1 buffer and returns to A's saved frame and address space.
7. A receives and verifies the two reply bytes.
8. A requests the fixed finish operation. EL1 checks the buffers, reject counters and every byte of both physical pages before printing the synthetic journal.

The caller's pointer and length are checked against the fixed alias page and limit before each user-memory copy; `checked_add` avoids wrapping. The actual byte operations use AArch64 loads/stores while the caller's TTBR is active. The negative calls also verify that both mailbox buffers remain empty. Tables are immutable, interrupts are masked, and the test has only one CPU. Root switches use a conservative full local TLB invalidation and barriers. The output is generated from the verified final state; it is not a tracing facility for arbitrary real I/O.

This is **not a general IPC API**: callers and phases are hard-coded, there is no authentication, dynamic memory-map validation, arbitrary bad-pointer fault recovery, general error handling, queueing, blocking scheduler, simultaneous execution, process lifecycle or certified security boundary. The special addresses and message bytes are synthetic. The user pages are also stack backing for this fixture; nothing here demonstrates general process isolation, DMA/IOMMU isolation, SMP, FP/SIMD preservation, hardware Apple support, or an operating system ready for real data. Serial output contains only the fixed scenario markers and values.

## Reproduce

Install Rust with target `aarch64-unknown-none-softfloat`, Python 3 and QEMU AArch64. From this directory:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/user-ipc-demo \
  --output-dir /path/to/output
```

The runner uses QEMU TCG with one Cortex-A57, no host directory sharing, no disks and no network. It checks the complete serial result and image hash before and after the run, and records the QEMU version. Do not rebuild the image during the run. The [recorded evidence](evidence/user-ipc-evidence.json) covers only this scenario.
