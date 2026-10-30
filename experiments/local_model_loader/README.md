# Bounded local model loader in QEMU

A Rust `no_std` AArch64 guest reads one synthetic sector from a **temporary read-only VirtIO block device**, validates it, and runs a small integer dense network. It uses no network, host-directory sharing or real disk. This is an independent demonstration model with hand-written coefficients: it is not trained, Jev or Laya, has no system-control capability and makes no claim about predictive quality or speed.

## Fixed model format

The input is exactly one 512-byte sector. All multibyte integers use little endian.

| Bytes | Meaning |
| --- | --- |
| 0–7 | Magic `NSDENSE1` |
| 8–9 | Version `1` as u16 |
| 10–11 | Reserved flags, zero |
| 12–15, 16–19, 20–23 | Input, hidden and output dimensions as u32; only `3,4,2` are accepted |
| 24–27 | Payload byte count as u32; must equal the shape-derived count, 32 |
| 28–59 | SHA-256 of bytes 0–27 concatenated with bytes 60–91 |
| 60–91 | 12 signed i8 first-layer weights, 4 signed i16 biases, 8 signed i8 second-layer weights, 2 signed i16 biases; row-major |
| 92–511 | Canonical zero padding |

The parser uses checked dimension arithmetic before verifying the exact shape, size and canonical padding. It verifies the digest before decoding coefficients or inferring. The hash is stored alongside the model: it detects accidental corruption but **does not authenticate the model or establish its provenance**. Anyone able to modify the model can calculate a new digest. Signature verification, trusted key distribution and rollback protection are absent.

The parser uses fixed-size stack arrays and allocates no model-dependent heap memory. The VirtIO driver uses separate fixed-size 64 KiB guest heap and DMA arenas with a one-shot allocator; this HAL assumes QEMU's single-CPU guest with MMU/caches disabled and identity-mapped memory and is not a native Apple DMA implementation. Only the synthetic QEMU block device is supported by this fixture.

The input is three integers each in 0–100. It computes a dense layer, ReLU and a second dense layer. Values and arithmetic are checked; for any permitted i8 weights and i16 biases, the fixed shapes and input bounds keep sums safely within i32 (the implementation nevertheless performs checked i64 arithmetic and checked conversion). The reference has four different observations and checks both intermediate hidden values and scores. The guest also checks input `101` is rejected. No probability or learned decision is produced.

## Tests and limits

The runner computes the expected reference independently in Python, builds a fixture with Python's SHA-256, and launches a fresh VM per case. The guest's own SHA-256 implementation checks `abc` and independent expected digests for lengths 0, 55, 56, 64 and 512, in addition to verifying the model digest. It rejects ten variants: magic, version, reserved flags, unsupported shape, overflowing dimensions, two invalid sizes, corrupted hash, corrupted payload and nonzero padding. The length mutations are logical sector-header tests; they are not physical disk truncation or fault-injection tests. Each image is created privately and QEMU is configured to attach it read-only; the guest makes no write calls and the runner checks the image unchanged. The guest does not check the device read-only feature bit. These are targeted fixtures, not exhaustive parser or cryptographic verification.

The runner records the exact synthetic serial output, reference results, QEMU version and SHA-256 of the guest and fixture; it verifies the guest hash before and after the run. The [modern transport evidence](evidence/local-model-modern.json) and [legacy transport evidence](evidence/local-model-legacy.json) cover this fixture only. The locally patched `virtio-drivers` dependency and its transport limitations are described in [virtio_fix](../virtio_fix/README.md).

## Reproduce

Install Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3, then run from this directory:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/local-model-loader \
  --output-dir /path/to/output
# Add --legacy to exercise legacy VirtIO MMIO.
```

Do not rebuild the guest while the runner is executing. Integration of a real local model would additionally require an actual distributable model and compatible inference runtime, resource accounting, authentication and evaluation; those are not implemented here.
