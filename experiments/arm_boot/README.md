# A first freestanding Rust kernel for an ARM virtual machine

This experiment is a **real freestanding AArch64 program** which starts in a
QEMU `virt` machine, initializes its stack and zeroes BSS, writes a marker to
the virtual PL011 serial controller, and waits. It does not use a host OS
inside the guest. It is **not** macOS, an Apple Silicon boot image, a full OS,
or a driver for any physical Mac hardware. It does not alter the host system
or its boot process. There is no memory manager, scheduler, interrupt handler,
process, filesystem, GPU, network, user interface, or MCP service yet.

## Build and run

Requires Rust and Cargo with the `aarch64-unknown-none-softfloat` standard core
target, Python 3, and `qemu-system-aarch64`. Build on a development host; run
the resulting kernel **inside QEMU only**:

```sh
cargo build --locked --offline --release
python3 run_vm.py --qemu qemu-system-aarch64
```

You may supply the absolute path to a separately installed QEMU with `--qemu`.
The runner starts a single ephemeral QEMU virtual machine with TCG software
emulation, one emulated Cortex-A57 CPU, 128 MiB RAM, serial output, no network
interface and no disk. It stops the virtual machine after the expected serial
message, an error, or a bounded timeout. An expected run prints:

```text
ARM Rust kernel: boot OK
PASS: expected serial output from the QEMU VM
```

## Observed run and scope

On the research Mac (M4 Pro, macOS 15.1.1), an isolated Rust 1.93.1 toolchain
with `aarch64-unknown-none-softfloat` compiled the image. A QEMU 11.1.3
Homebrew ARM64 Sequoia bottle was downloaded with Homebrew to a workspace cache,
along with the missing library bottles; the binaries were extracted and
relocated within the workspace, and linked against pre-existing Homebrew
libraries as needed. Neither Rust's target nor QEMU was installed globally.
The original QEMU bottle was obtained through Homebrew's checksum-verified
download; the relocated executable is re-signed locally and is not itself a
publisher-signed artifact. This repository does not include the downloaded
toolchain, emulator, compiled image or third-party binaries.

The actual VM invocation is equivalent to:

```sh
qemu-system-aarch64 -M virt,virtualization=off -accel tcg \
  -cpu cortex-a57 -smp 1 -m 128M -nic none -nographic \
  -monitor none -serial stdio -no-reboot \
  -kernel target/aarch64-unknown-none-softfloat/release/arm-boot
```

The virtual serial output contained the expected boot marker. The serial
controller address, RAM address and execution assumptions are specific to the
QEMU `virt` machine. The `unsafe` code accesses its memory-mapped device and
executes a wait instruction. The soft-float target and disabled NEON reduce
SIMD assumptions, but we have not formally audited every instruction or built
a context-switch implementation. Failure handling is deliberately minimal.

This is a first boot foundation: the virtual machine allows controlled OS
development, but says nothing about real M4 Pro firmware, boot, drivers,
performance or macOS compatibility.

Primary references: [Rust bare-metal AArch64 target](https://doc.rust-lang.org/rustc/platform-support/aarch64-unknown-none.html),
[QEMU virt board](https://www.qemu.org/docs/master/system/arm/virt.html).

### Isolated Rust setup used here

The following setup uses an existing `rustup` executable as the installer and
places the additional toolchain entirely in a chosen workspace folder. Set
`TOOLS` to an **absolute path outside this repository**; it will download the
official Rust 1.93.1 components into that folder. It does not update the user's
default toolchain or shell configuration.

```sh
mkdir -p "$TOOLS/bootstrap"
cp "$(command -v rustup)" "$TOOLS/bootstrap/rustup-init"
RUSTUP_HOME="$TOOLS/rustup" CARGO_HOME="$TOOLS/cargo" \
  "$TOOLS/bootstrap/rustup-init" -y --no-modify-path \
  --default-toolchain 1.93.1 --profile minimal \
  --target aarch64-unknown-none-softfloat

RUSTUP_HOME="$TOOLS/rustup" CARGO_HOME="$TOOLS/cargo" \
  CARGO_TARGET_DIR="$TOOLS/build" \
  "$TOOLS/cargo/bin/cargo" build --locked --offline --release
python3 run_vm.py --qemu "$QEMU" --image \
  "$TOOLS/build/aarch64-unknown-none-softfloat/release/arm-boot"
```

Here `QEMU` is the absolute path to an available QEMU binary; the commands
above do not install QEMU. A plain Cargo/QEMU installation is equally usable.
On the original host, both independent Rust builds produced a 66,616-byte ELF
image with SHA-256
`ca7f5d28be835e37328932b8f52bd030f895c533e20ad6e9ee975617b34fa6be`.
Identical hashes are not guaranteed across host toolchains or future rebuilds.
