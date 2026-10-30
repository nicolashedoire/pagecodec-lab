# Synthetic workspace persistence across a VM restart

This QEMU-only Rust guest connects `workspace_bridge` to an emulated VirtIO block device. It creates a fixed synthetic base document, applies a human title edit and an assistant status edit to separate in-memory copies, checks their merged origins, and serializes the **values read from those copies** into a versioned 512-byte sector. The guest calls the driver’s `flush()` and verifies a byte-for-byte readback. A successful `flush()` return does not prove a FLUSH command was issued: the driver can return success when the feature was not negotiated. After the guest reports success, the runner sends SIGTERM to QEMU, changes the synthetic control sector on the stopped fixture, and starts a new VM instance over that image. This is not an autonomous guest reboot or a graceful shutdown. The second guest validates and decodes the two stored values, rebuilds the base, actor-to-copy mapping and copies from hard-coded definitions, and verifies the reconstructed values and merge origins. The base, mappings and origin labels themselves were not stored in the record.

The runner alone creates a new private temporary directory and 1 MiB raw images with exclusive creation. It has no option to attach an existing disk or image. It sets a synthetic control sector to select save or restore; the guest writes only sector 4. The runner checks the rest of the fixture is unchanged, then tests two additional freshly created images: one with an invalid checksum, another with an invalid length but a recomputed checksum. The guest rejects both, and the runner verifies those images were not changed. The [recorded evidence](evidence/workspace-persistence-evidence.json) contains the guest hash, QEMU version and synthetic serial I/O outcomes.

The record holds two fixed fields only: it does not persist arbitrary documents, edit history, conflicts, actor identities, graphical workspaces or real files. FNV-1a checks for incidental corruption; it is not authentication or a cryptographic integrity guarantee. The guest accepts only the known synthetic fixture values. The driver call and host-orchestrated restart exercise do not prove atomic updates, crash recovery, durable storage after power loss, process isolation, confidentiality or authorization. This one-CPU QEMU HAL has the MMU and caches disabled and must not be used for native DMA. The guest uses the [local `virtio-drivers` 0.13.0 patch](../virtio_fix/README.md), which checks modern `FEATURES_OK` acceptance and propagates refusal; this persistence experiment uses accepting QEMU devices and does not itself test a refusing block device. Legacy-mode detection assumes the provided MMIO/PCI transports, and asynchronous reset remains unhandled.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-persistence-demo \
  --output-dir /path/to/output
```

The host harness creates synthetic images, starts QEMU and reads their bytes after each VM exits. The OS code and bridge execute in the VM. The default is modern VirtIO-MMIO; add `--legacy` to test the unchanged legacy path, using a separate output directory to retain both logs. Do not rebuild the guest while the runner is active; it checks the image hash before and after its run. No network, host block device or shared host directory is attached. A [legacy-mode synthetic run](evidence/workspace-persistence-legacy-evidence.json) is also recorded.
