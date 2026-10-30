# Two-slot synthetic workspace recovery in a graphical QEMU guest

This isolated experiment extends a copy of [`graph_persistence`](../graph_persistence/), using the locally [patched VirtIO driver](../virtio_fix/). The graphical QEMU guest exposes MCP operations on a synthetic base and two copies. It writes snapshots to two 4,096-byte slots on a virtual block device. The runner exclusively creates disposable 1 MiB images in a private temporary directory; it has no option to attach an existing image or host block device.

A snapshot contains the full synthetic base, human copy and assistant copy. The guest reconstructs its bridge and recomputes merge origins and conflicts after loading. Actor identities and bindings are fixed labels, not authenticated credentials; original edit history and origin labels are not persisted. Snapshot limits apply on save/load: at most 16 fields per copy/base, keys up to 32 UTF-8 bytes, values up to 64 bytes, and a JSON payload of at most 3,584 bytes. Edits can succeed in memory yet exceed a later save limit.

## Slot and recovery protocol

Slots occupy sectors 4–11 and 12–19. Each has a metadata sector followed by seven payload sectors. The metadata includes a versioned magic, nonzero 64-bit generation, bounded payload length, a non-cryptographic FNV checksum over generation, length and payload, and a commit byte. Reserved bytes and payload padding must be zero. A committed slot is a candidate only after complete length, checksum, JSON and document validation. When both candidates are valid, the guest selects the higher generation; equal valid generations cause an explicit error. An invalid or staged slot is never used as a committed snapshot. When neither slot is valid, only two completely blank slots are treated as a fresh store; otherwise load and save return errors rather than silently resetting it.

`document_save` accepts `{"mode":"commit"}` or the **test-only failpoint** `{"mode":"stage"}`. The guest selects the slot other than the latest valid one, writes the complete new record with its commit byte clear, and calls the driver's `flush()`. In commit mode it then writes the metadata sector with the commit byte set and calls `flush()` again. In stage mode it returns before that commit-sector write. A later `mode=commit` request **does not finalize the earlier staged bytes**: it serializes the then-current in-memory snapshot and rewrites the inactive slot before committing. Edits made between the two requests can therefore change what gets committed. `document_load` validates both candidates first and only replaces in-memory state after selecting and reconstructing a valid snapshot.

This protocol and test **do not prove atomic writes or durable recovery after power loss**. A successful `flush()` call does not prove the FLUSH feature was negotiated or a flush command reached a device. The test ends QEMU only after the stage operation has returned; it does not interrupt QEMU during a write, model torn sectors or device caches, or simulate physical power loss. A checksum can miss corruption and is not authentication. The two-slot strategy has no authenticated metadata, concurrent-writer protection, generation-wrap recovery or general transaction semantics. Missing/read-only disks, failed writes, unexpected feature negotiation, simultaneous writers and arbitrary malicious records are not exercised. JSON duplicate keys are not separately rejected. No native Apple hardware, DMA safety, real process isolation or performance claims follow.

## Tested sequence

The [modern-MMIO evidence](evidence/graph-crash-recovery-evidence.json) and [legacy-MMIO evidence](evidence/graph-crash-recovery-legacy-evidence.json) contain synthetic MCP requests/responses and QMP framebuffer hashes for eight QEMU instances per run:

1. Create and commit generation 1 with two copies and a retained conflict.
2. In a new guest, load generation 1, edit a copy, stage generation 2 and return before its commit byte is written. The runner terminates QEMU and verifies generation 1's slot and unrelated sectors remain unchanged.
3. In another guest, load generation 1, reproduce the edit and commit generation 2. A subsequent guest loads generation 2 and verifies the full snapshot, conflict and origin labels.
4. On a separately created copy of the synthetic image, corrupt the newest slot's payload and confirm a new guest falls back to generation 1. When both slots are corrupted, load fails without modifying the active in-memory snapshot or disk.
5. On a new blank synthetic image, stage its first snapshot and terminate QEMU. A subsequent guest rejects both load and save without overwriting that incomplete slot.

These are host-orchestrated QEMU process stops on synthetic fixtures. The graphical panels are drawings, not separate isolated applications, and edits labeled human are synthetic MCP inputs. The guest has a finite non-reclaiming allocator and per-boot request limit. Its QEMU-specific DMA HAL assumes MMU/caches disabled and must not be reused for real hardware.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3, from this directory:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/graph-crash-recovery-demo \
  --output-dir /path/to/output
# Optional: repeat with --transport legacy
```

The host harness sends synthetic MCP messages and inspects only fixtures it created. All guest component execution is inside QEMU. The runner logs guest hash, QEMU version, slot hashes, MCP I/O and framebuffer hashes, and rejects a binary that changes during the run.
