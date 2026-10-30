# Interactive panels and two-slot snapshots in one QEMU guest

This experimental guest combines copies of [`graph_interaction`](../graph_interaction/) and [`graph_crash_recovery`](../graph_crash_recovery/) in a separate binary, without changing either original experiment. It uses the local [patched VirtIO driver](../virtio_fix/), a virtual GPU and tablet, a synthetic MCP server, an in-memory workspace bridge and a virtual block device. The guest has two drawn panels and two controls per panel, with separate logical focus and click counts for human and assistant labels.

A human-labeled pointer can activate buttons through VirtIO tablet press events; `move_ai` and `click_ai` move and activate an assistant-labeled pointer via MCP. The click changes only the demo's logical focus and counter. **Buttons do not edit or save documents.** The separate document operations accept synthetic MCP edits, even when labeled human; they can save and load the base and both document copies using the two-slot protocol. Interaction state is not persisted and starts fresh in a new guest. The panels are not real isolated applications, windows or keyboard focus. Labels and the MCP endpoint are not authenticated, and the pointer and document registries are separate.

## Snapshot and recovery scope

The disk contains two 4,096-byte slots at sectors 4–11 and 12–19. A save writes the inactive slot without a commit marker and then, for `mode=commit`, writes its metadata sector with the marker. `mode=stage` intentionally stops before the marker. **A later `mode=commit` request serializes the then-current state and rewrites the slot; it does not finalize a prior staged snapshot.** Loading chooses the highest completely validated committed generation, or rejects the request when no committed candidate is valid. A valid older slot is retained during staging of the newer one.

Records store the actual synthetic base and both copies; actor bindings are reconstructed from fixed labels and conflict/origin results are recomputed. Original event history is not stored. Limits are 16 fields in each base/copy, 32 UTF-8 bytes per key, 64 per value and 3,584 bytes of JSON payload. Edits can succeed but a later save can reject an oversized snapshot. The guest validates versioned magic, lengths, generation, checksum, padding and document shape before replacing active state. FNV is not cryptographic authentication, and duplicate JSON member names are not separately rejected.

## QEMU scenario

The [modern-MMIO evidence](evidence/graph-unified-evidence.json) and [legacy-MMIO evidence](evidence/graph-unified-legacy-evidence.json) record synthetic MCP and QMP input requests/responses, selected snapshots and hashes. Each run uses eight VM instances over disposable runner-created images:

1. In the first guest, a synthetic QMP tablet press activates human `ACTION`; an MCP move and click activate assistant `SELECT` without changing the human logical state. Before/after framebuffer hashes differ for both interactions. The test then applies synthetic document edits and commits generation 1 with a retained conflict.
2. A new guest verifies the interaction state is fresh, loads generation 1 and stages generation 2. The host terminates QEMU **after the stage response and before commit**; it checks that the old slot and unrelated sectors are unchanged.
3. A new guest selects generation 1, recreates the second edit and commits generation 2. Another guest restores generation 2 and verifies the complete copies, conflict and recomputed origins.
4. Corrupting the newest slot on a separate synthetic image results in fallback to generation 1. Corrupting both causes an error and leaves the active document and disk unchanged. A separately staged first-ever snapshot is also rejected on restart without being overwritten.

Framebuffer hashes establish a change, not the exact geometry or origin of every changed pixel. QMP tablet events are synthetic; the test does not demonstrate a physical person's input. The runner creates images exclusively in a private temporary directory and accepts no existing disk or physical device.

The explicit stage failpoint and host SIGTERM do not model interruption during an individual write, torn sectors, power loss or device caches. A successful `flush()` return does not establish that FLUSH was negotiated or a command reached the device. An MCP click mutates its logical state before GPU flush; if that flush fails, the guest reports an error and halts without rolling back the click. There is **no claim of atomicity, crash durability, authentication, confidentiality or native Apple hardware support**. Missing or read-only disks, failed writes, malicious data, concurrent writers, event-history recovery and actor identity enforcement are not covered. The QEMU-specific DMA HAL and finite non-reclaiming allocator are not suitable for real hardware or production use.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/graph-unified-demo \
  --output-dir /path/to/output
# Optional: repeat with --transport legacy
```

The host harness requires an empty output directory so it will not overwrite existing captures. It starts QEMU, sends synthetic MCP/QMP requests, inspects only images it creates, and logs the guest image hash, QEMU version, slot hashes, framebuffer hashes and protocol I/O. On failure, any available QEMU diagnostic output is sent to standard error. Guest code executes inside QEMU; the binary hash is checked before and after the run.
