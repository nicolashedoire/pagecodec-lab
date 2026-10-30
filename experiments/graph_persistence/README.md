# Graphical guest: synthetic workspace snapshot across QEMU instances

This Rust/assembly QEMU experiment starts from a copy of [`dual_pointer_display`](../dual_pointer_display/), including its virtual GPU, tablet, pointer router and experimental MCP server. It adds an emulated VirtIO block device and three MCP tools: `document_snapshot` returns the synthetic base and both independent copies, `document_save` writes them to a bounded disk record, and `document_load` validates a record and reconstructs the active bridge. The original graphical guest and bridge sources are not modified by this experiment.

The bridge itself has no base-export API. This guest maintains its own immutable base beside the bridge, creates the bridge from that base, and reads both full copies through its public view API. Loading parses a new base and copies, creates a separate bridge, reapplies additions, changes and deletions relative to that base, and only then replaces the active document. The snapshot stores **actual base and copy contents**. Actor labels and bindings are recreated from fixed definitions; event history, revision information and origin labels are not stored. The guest recomputes merge origins and conflicts from the restored documents. The reconstructed bridge events are newly generated from differences and are not the original edit history. Labels are not authenticated.

The one-slot record spans 4,096 bytes starting at sector 4. Its versioned magic, payload length, non-cryptographic FNV checksum, zero padding, JSON structure, maximum 16 fields per document and UTF-8 byte limits (32 for keys, 64 for values) are checked before replacement. Individual edits do not enforce the 16-field document limit or the 4,080-byte serialized-payload limit, so editing can succeed and a later save can reject the resulting snapshot without writing it. This is a small synthetic format, not an authenticated or adversarially hardened parser; duplicate JSON member names are not separately rejected. Checksums do not establish authenticity. The guest rejects a missing or unexpected-capacity disk and reports save/load errors over MCP. A failed in-place save can damage the sole stored record; there is no transaction, atomicity or recovery protocol.

The [recorded QEMU evidence](evidence/graph-persistence-evidence.json) contains full synthetic MCP request/response traces and framebuffer hashes from five guest instances configured for modern MMIO. A [second run](evidence/graph-persistence-legacy-evidence.json) repeats the same scenarios configured for legacy MMIO:

1. A first guest receives synthetic MCP edits to both copies, including an overlapping title conflict and a deletion; it saves the full snapshot. The runner checks the stored record and that sectors outside its assigned area are unchanged.
2. A new guest starts with a fresh in-memory base, loads the same runner-created image, and verifies the entire base and both copies, recomputed conflict and origins, and unchanged pointer state.
3. Three further guests reject a corrupted checksum, a structurally incomplete snapshot with a valid recomputed checksum, and an invalid length. In each case the test checks that the in-memory snapshot and disk remain unchanged.

The runner creates each 1 MiB image exclusively in a private temporary directory and does not accept an existing image or physical device as input. It terminates QEMU after each scenario and starts a new QEMU instance on the same temporary image for the restore; this is not an autonomous guest reboot. All edit inputs are synthetic MCP requests, including those labeled human. Pointer panels are drawings, not real isolated applications, and actor labels and the local serial endpoint provide no authentication or authorization.

The scenario does not exercise missing or read-only disks, write errors, interrupted saves or feature-negotiation rejection. A successful `flush()` return does **not** establish that FLUSH was negotiated or a device command was issued. The experiment does not prove crash consistency, power-loss durability, storage ordering, recovery, confidential or authenticated data, a real graphical editor, host document access, native Apple hardware, or performance improvements. It uses the local [patched `virtio-drivers` 0.13.0](../virtio_fix/) copy: modern feature negotiation is checked in that patch, while the legacy path retains its prior behavior. These tests exercise QEMU accepting the devices; they do not test a GPU, input or block device refusing negotiation. The DMA HAL assumes this exact QEMU guest with MMU and caches disabled and is unsuitable for physical DMA. The guest has a non-reclaiming finite allocator and a 64-message per-boot limit; it is not hardened for long sessions or untrusted callers.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3, from this directory:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/graph-persistence-demo \
  --output-dir /path/to/output
# Optional: repeat with --transport legacy
```

The host harness launches QEMU, transports the synthetic MCP messages and inspects only images it created. The component code executes in the VM. It records the guest image hash, QEMU version, MCP I/O and QMP framebuffer hashes. Do not rebuild the guest while the runner is active; its binary hash is checked before and after the run.
