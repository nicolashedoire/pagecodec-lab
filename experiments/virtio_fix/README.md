# Local VirtIO feature-acceptance patch

This experiment vendors and locally patches the MIT-licensed [`virtio-drivers` 0.13.0](https://crates.io/crates/virtio-drivers/0.13.0) source. Its copy of the [upstream license](vendor/virtio-drivers/LICENSE) and README are retained. The unmodified crates.io archive has SHA-256 `cfdc1c628cdd8ce7c3b9e65a8ed550d0338e9ef9f911e729666f1cce097de2f7`; `Cargo.lock` pins the other dependency versions. This is an experimental private fork, not an upstream release.

For modern devices, [VirtIO 1.2 §3.1.1](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html) requires a status read after setting `FEATURES_OK` and forbids continuing initialization if the bit is clear. The upstream 0.13.0 `Transport::begin_init` sets the bit but does not read it back. The local patch reads status before continuing, sets `FAILED` and returns `FeatureNegotiationFailed` if `FEATURES_OK` is absent or `DEVICE_NEEDS_RESET` is present. Nine device constructors now propagate the returned error with `?`. This experiment's RNG guest links the patched copy directly.

**This is an API break:** `Transport::begin_init` changes from returning `F` to `Result<F>`. Any other caller or custom implementation must be updated and reviewed. No other experiment or shared dependency was changed; other guests do not acquire this fix automatically. Migration would require an explicit local dependency or Cargo patch and integration tests for each consumer. Do not publish this fork as an unmodified upstream version.

## Evidence and limits

The [recorded VM evidence](evidence/virtio-fix-evidence.json) covers four QEMU runs. In each, an in-guest **simulated transport** refuses `FEATURES_OK`: the patched, real `VirtIORng::new` returns `FeatureNegotiationFailed`, with no RNG queue calls, DMA allocation or `DRIVER_OK` attempt. A second mock sets `DEVICE_NEEDS_RESET` while retaining `FEATURES_OK`; the RNG constructor also returns the error without initializing a queue. These counters establish behavior for this RNG constructor and these mocks only. In particular, upstream `VirtIOInput::new` allocates a buffer before negotiating, so the RNG's no-allocation observation cannot be generalized.

With actual QEMU VirtIO-MMIO RNG devices, the patched RNG constructor and request path successfully read 32 deterministic fixture bytes in both modern and legacy modes. Separate negative cases confirm wrong fixture bytes and a missing device are detected. The refusal and reset conditions are mocked, not delivered by a real rejecting QEMU or physical device. Both base and `alloc` feature configurations compiled; other device constructors were updated and compiled, but not exercised in this experiment. These cases do not establish general absence of regressions or complete VirtIO conformance.

The patch uses `requires_legacy_layout()` to distinguish the supplied MMIO and PCI transports: the acceptance check is skipped for legacy layout to preserve existing behavior. The method describes queue layout, and arbitrary third-party transports may not equate it with negotiation mode; they require separate review. Legacy initialization is otherwise unchanged, including upstream status writes. Reset completion is not polled, ordering and feature dependencies are not comprehensively checked, and device-specific failure cleanup and concurrent or adversarial devices need further testing. Do not use this prototype on physical hardware or as a production security boundary.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo fetch --locked
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/virtio-fix-lab \
  --output-dir /path/to/output
```

The guest runs only in isolated QEMU TCG VMs with no disks, network or host directory sharing. The harness creates and removes temporary deterministic RNG fixture files on the host filesystem, then records the guest hash, QEMU version and synthetic serial output. No physical block device, real entropy or personal data is attached. Do not rebuild the guest image during the run. Other third-party dependencies retain their own licenses; inspect them before redistributing a binary.
