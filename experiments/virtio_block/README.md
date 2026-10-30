# Synthetic VirtIO block guest

This is a single-purpose AArch64 Rust `no_std` guest for QEMU `virt`, not a filesystem, production block driver, or bootable operating system. It uses [`virtio-drivers` 0.13.0](https://github.com/rcore-os/virtio-drivers) and a simple single-CPU identity-mapped HAL with MMU and caches disabled. It finds a VirtIO block device, checks its size is 2,048 sectors, reads two zero sectors, writes 1,024 deterministic bytes to sectors 4–5, calls `flush()`, verifies exact readback, and checks neighboring sectors. Serial output contains synthetic information only.

`run_demo.py` creates two new 1 MiB image files exclusively in a temporary private directory and starts two isolated QEMU instances. It does not accept a disk path, expose host directories, attach a host block device or use networking. The images are backed by the host filesystem and deleted afterward; this does use temporary host storage. The writable run checks the entire image against the expected bytes after the VM terminates; the read-only-configured run verifies that the guest reports a write error and that its image remains entirely zero. The error is not classified, so the run alone does not establish why the write failed. Nothing is installed on the host OS.

The `virtio-drivers` `flush()` API returns success even if the device did not negotiate flush support; this experiment verifies the call returned successfully and the data could be read back, not that a flush request was actually sent or that writes would survive a power failure. `virtio-drivers` 0.13.0 also does not recheck `FEATURES_OK` after feature negotiation. The HAL is limited to this simulated one-CPU machine and does not establish support for real Apple storage hardware, filesystems, concurrency, isolation, durability, or host-device safety outside this harness. Do not use this guest with any existing image or physical disk.

Build with Rust, the `aarch64-unknown-none-softfloat` target and Cargo, then run the standard-library Python harness with an installed `qemu-system-aarch64`:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/virtio-block-lab \
  --output-dir /path/to/output
```

The output `virtio-block-evidence.json` includes the ELF and image hashes, both synthetic serial logs, and the host-side verification result. The guest executes only inside QEMU; Python orchestrates QEMU and checks the disposable image bytes.

The [recorded synthetic run](evidence/virtio-block-evidence.json) contains the writable and read-only trial logs. It demonstrates the observed readback and error handling for these two fixtures; it is not proof of durability after power loss.
