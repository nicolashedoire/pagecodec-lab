# Bounded synthetic I/O journal

This experiment serializes and inspects **fictional events** in a QEMU guest. It does not observe or log actual machine I/O. A `no_std` library stores at most 16 entries in a fixed array. Each entry contains only a sequence number and fixed enum values for category, synthetic actor and outcome; there is no free-form payload. Appending a seventeenth entry returns `Full` without replacing an earlier entry. Sequence numbers start at 1 within this one journal, not a global clock or durable identity.

The canonical 512-byte sector has an eight-byte versioned magic, a count, reserved zero bytes, up to 16 fixed eight-byte records and a trailing FNV-1a checksum. Decoding validates length, magic, checksum, count, reserved bytes, consecutive sequence numbers and enum values. FNV detects accidental corruption; it is **not authentication or cryptographic integrity**. The guest accumulates a complete journal in memory and writes one sector; this is not an append-on-disk transaction protocol. “Replay” means decoding and inspecting records after a VM restart, **not re-executing I/O actions**.

The runner creates a private temporary 1 MiB image exclusively for each scenario. In the save run, the guest appends 16 fictional events, verifies that a seventeenth is refused, writes sector 4, calls `flush()` and checks readback. After the runner stops QEMU and changes a synthetic control sector, a new QEMU guest reads and validates that journal. The runner compares the whole temporary image with independently constructed expected bytes and checks that replay did not modify it.

Four negative fixtures verify rejection without image mutation: corrupted checksum; **logical truncation** by passing a 511-byte slice of a complete sector to the decoder; count 17 with a recomputed checksum; and a repeated sequence with a recomputed checksum. The disk image is not physically truncated. These cases do not exhaustively cover invalid enum/padding values, device errors, torn writes or all possible corruptions. The driver `flush()` can report success when flush was not negotiated, so neither the call nor this host-orchestrated restart proves durability after power loss or atomic updates. There is no tamper resistance, authenticated actor identity, process isolation or universal I/O logging.

The guest uses the [local patched VirtIO driver](../virtio_fix/README.md) in QEMU. No host block device, real device trace, private content, network or host directory is attached. Temporary fixture files do use the host filesystem and are deleted after the runs. The guest's single-CPU, caches-disabled HAL is not suitable for native DMA or Apple hardware.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/io-journal-sketch \
  --output-dir /path/to/output
# Optional: repeat with --legacy and a separate output directory
```

The default uses modern VirtIO-MMIO; `--legacy` tests the separate legacy path. The runner hashes the guest before/after, records QEMU version and synthetic serial output, and deletes the fixture images. Do not rebuild the guest during execution. The [modern](evidence/io-journal-evidence.json) and [legacy](evidence/io-journal-legacy-evidence.json) logs document these specific tests, not general reliability or security.
