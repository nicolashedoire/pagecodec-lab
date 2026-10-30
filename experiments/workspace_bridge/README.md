# Identity-to-copy bridge

This `no_std` + `alloc` Rust experiment connects two existing components: `actor_core` represents actors and space assignments, and `system_core` provides two document copies and a deterministic field-level three-way merge. The bridge binds one human ID and one assistant ID to distinct assigned spaces and separate snapshots of the same base document. Each edit or view requires the bound actor ID and its assigned space. A successful edit records the actor, space, key and optional value; rejected edits do not produce an event. The bridge can map the merge's human, assistant and both provenance labels back to the bound IDs.

Compatible independent edits merge with provenance. Conflicting edits preserve the base and both proposed values, and materializing a complete result is rejected until the conflict is resolved. Deletions retain tombstones and attribution. The merge compares fields and does not understand the meaning of a document; an apparently conflict-free result is not guaranteed to be semantically correct. The provenance labels describe net changes from the common base, not the complete event history. Events remain available separately in memory.

The bridge currently supports **one human and one assistant copy**; a third actor, including a remote actor known to the registry, is rejected as unbound. It has no automatic conflict resolver, AI model, MCP connection, persistence, file system, real UI, or integration with the graphical demo. It does not authenticate its actor IDs or establish OS-enforced isolation: callers supply labels, and the caller with access to the bridge can invoke merge. This is not an authorization boundary. Events are neither durable nor bounded and may contain document data. The bridge takes ownership of the registry at creation and exposes no operation to change assignments afterward.

## Reproduce

From this directory, with Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3 installed:

```sh
cargo build --release --locked --features qemu-demo
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-bridge-demo \
  --output-dir /path/to/output
```

The host harness only starts QEMU and collects its serial stream. The Rust guest runs inside a single-CPU QEMU TCG VM without network, disks or host directory sharing. The synthetic scenario covers binding validation, independent edits, provenance bound to IDs, rejected wrong-space and remote edits, deterministic re-merge, conflicting versions, deletion attribution and identical edits attributed to both. The guest uses a fixed 128 KiB non-reclaiming demo allocator. The harness records the guest image hash before QEMU starts and the QEMU version; do not rebuild the image during the run. The [recorded synthetic log](evidence/workspace-bridge-evidence.json) is evidence for this scenario, not a claim of secure isolation or general correctness.
