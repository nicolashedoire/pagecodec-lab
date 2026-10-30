# A Rust VirtIO entropy driver in QEMU

This is a **freestanding AArch64 guest**, derived from this repository's
[initial ARM boot](../arm_boot/README.md), that adds a VirtIO entropy-device
driver. It uses [virtio-drivers 0.13.0](https://crates.io/crates/virtio-drivers/0.13.0),
a separately maintained MIT-licensed `no_std` Rust library, via its MMIO
transport and RNG driver. All dependencies and checksums are pinned in
`Cargo.lock`; the library code is not vendored in this repository. Consult
each dependency's license when redistributing a compiled artifact.

The guest also links the local [`system_core` library](../system_core/README.md)
using its `no_std` and `alloc` interfaces. This combines actual library code
and the VirtIO driver inside one guest address space; it does not implement
process isolation, windows, access-control enforcement, an AI model, or AI
conflict resolution.

This is a **virtual device**, not a driver for the Mac's physical GPU, SSD,
USB controller or entropy hardware. The guest has no general memory manager,
interrupt handler, storage, network, system services or macOS compatibility.
It must be run only on the specified QEMU `virt` machine, not booted on a Mac.

## What it does

The guest starts with one virtual CPU, a stack and zeroed BSS. Before probing
the device, it checks two fictional in-memory workspace cases in every VM:
non-conflicting edits preserve human and assistant origin, and a conflicting
edit preserves both values and cannot be materialized as a finished merge.
Both copies live in a single kernel address space and are not independent OS
processes. The guest emits fictional inputs and check results over serial.

It then probes QEMU's
32 VirtIO-MMIO windows, finds the entropy device, initializes its VirtIO queue
and requests 32 bytes. The host test harness supplies a **deterministic fixture
file**, not confidential user data or a source of usable cryptographic entropy.
The guest prints the received byte count and hex bytes over the virtual PL011
serial port, compares them with the expected sequence, and emits a pass/fail
marker. **Never attach real entropy or secret data to this lab image:** its
serial output and JSON evidence intentionally disclose every received byte.
Never use this example as a random-number generator or for security-sensitive
operations.

The test harness runs three fresh virtual machines:

1. Correct fixture: the guest obtains and verifies the expected 32 bytes.
   It uses those actual received bytes as the assistant's fictional edit,
   merges them with a human edit, and verifies merged values and origins.
2. Different fixture: the guest reports a data mismatch, which is the expected
   **negative-test success**; it must not emit an I/O-to-merge success marker.
3. No VirtIO entropy device: the guest reports that no recognized device was
   found. An unsupported or malformed transport can produce the same result;
   this does not prove physical absence and must not emit an I/O-to-merge marker.

On the research Mac, QEMU 11.1.3 in TCG mode and Rust 1.93.1 produced these
abbreviated lines; the structured evidence has the complete serial output:

```text
Valid synthetic source:
RECEIVED count=32 hex=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
IO_MERGE_CHECK human=human assistant=device_data data_hex=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
PASS: synthetic bytes via VirtIO RNG
Wrong synthetic source:
RECEIVED count=32 hex=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
FAIL: unexpected RNG data
Missing device:
PASS: no recognized VirtIO entropy device
```

The harness waits for the complete terminal marker, verifies all workspace
inputs and checks, checks exact device bytes, and requires the I/O-to-merge
marker exclusively for the valid case. It treats missing or unexpected markers
as failure. The negative fixture deliberately triggers the guest's
`FAIL` marker; this is not a failing test run. These checks establish one path
through an existing virtual device driver and workspace library; they do not
establish device security, randomness, performance, an AI decision, or a
complete operating system.
The [recorded JSON evidence](evidence/qemu-smoke.json) includes the synthetic
input descriptions, sizes and SHA-256 hashes, device presence, captured output,
expected markers, verification results, VM configuration, image hash and
timings for all three runs, plus the exact guest-reported count and hex values
where a device is present. The complete console capture ends when the harness
stops the VM. QEMU reads a temporary synthetic file on the host as its RNG
backend; this is not a trace of physical hardware or of every internal I/O.

## Reproduce

Requires QEMU with `qemu-system-aarch64`, the Rust AArch64 soft-float target,
Cargo and Python 3. Download dependencies once, then build reproducibly from
the lockfile; the second command does not use the network:

```sh
rustup target add aarch64-unknown-none-softfloat
cargo fetch --locked
cargo build --locked --offline --release
python3 run_vm.py --qemu qemu-system-aarch64
```

Run from this directory. `--image` accepts a separately built kernel ELF if
the Cargo target directory is outside this project. The runner creates only
temporary synthetic fixtures; it configures no network or disk for the guest.
By default it writes a new JSON report under ignored `run-results/`; use
`--report PATH` to choose a different report destination.
QEMU itself and third-party libraries are not included here. Setup of the
isolated toolchain used on the research Mac is explained in the
[ARM boot report](../arm_boot/README.md).

## Important implementation limits

The VM-only HAL assumes identity-mapped addresses, one CPU and the QEMU boot
environment, with a small fixed heap and page-aligned DMA pool. The guest
checks that it starts at EL1 with its MMU and data cache disabled before BSS
initialization or device access, otherwise waits without printing; the runner
reports a timeout if the expected result is missing. DMA allocations and shared
buffers are checked against the configured 128 MiB guest RAM range. Allocations are
never reused in this one-shot process. It does not implement cache coherency,
IOMMU mappings, robust multi-threading, normal deallocation or hardware
isolation. Interrupts remain disabled; the library's request blocks waiting for
the virtual device, so the host runner enforces a timeout. This unsafe code and
the fixed memory map are not appropriate for physical hardware or production.

The reviewed `virtio-drivers` 0.13.0 `Transport::begin_init` writes the
`FEATURES_OK` status but does not re-read it before proceeding, whereas the
[VirtIO specification](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html)
describes checking acceptance. This test exercises QEMU's accepting path only;
we have not tested a device that rejects negotiated features and do not claim
full specification compliance. This implementation needs further work before
being used with untrusted or physical devices.

References: [Rust bare-metal AArch64 target](https://doc.rust-lang.org/rustc/platform-support/aarch64-unknown-none.html),
[QEMU `virt` board](https://www.qemu.org/docs/master/system/arm/virt.html),
[VirtIO driver crate documentation](https://docs.rs/virtio-drivers/0.13.0/virtio_drivers/).
