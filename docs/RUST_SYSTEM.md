# Building the proposed system in Rust

Rust is the selected primary language for this project. The aim is a complete
system, including boot, kernel, drivers, services, human and AI workspaces,
MCP-facing control, model integration and a graphical/voice interface. The
current code is the beginning of that work, not a usable macOS replacement.

See the [candidate Rust components](RUST_COMPONENTS.md) for existing drivers,
libraries, services and an experimental AArch64 system under evaluation.

Rust supports [bare-metal AArch64](https://doc.rust-lang.org/stable/rustc/platform-support/aarch64-unknown-none.html)
without the full standard library. Real systems already use Rust: [Redox](https://github.com/redox-os/redox)
is an operating-system project, while Rust for Linux documents an
[Apple AGX GPU driver](https://rust-for-linux.com/apple-agx-gpu-driver).
That driver is a Linux component, not a drop-in driver for our system or
proof of M4 Pro support. Asahi's [M4 status](https://asahilinux.org/docs/platform/feature-support/m4/)
shows substantial unfinished platform work.

## Language and performance

- Use Rust for the kernel, drivers and services wherever practical. Minimal
  assembly may still be necessary for entry and processor instructions, and
  existing C interfaces or libraries may require carefully bounded FFI.
- Hardware access, DMA, concurrency and FFI require explicit unsafe boundaries.
  Rust reduces some classes of errors; it does not by itself guarantee that a
  driver is correct, secure, fast or compatible with firmware.
- Performance is a primary goal. Measure latency, throughput, memory,
  CPU consumption and overhead for end-to-end workloads. Safety and resource
  limits are design constraints of a functioning system, not evidence that
  performance has already improved.
- The owner requests **all subsequent project tests in virtual machines**.
  A VM is appropriate for functional development and controlled comparisons,
  but virtual-device timing does not establish native M4 Pro performance.

## Initial development path

1. Boot a minimal AArch64 Rust kernel in a generic virtual machine, with a
   serial console. A booting kernel only establishes the entry path.
2. Add exception handling, memory mappings, a physical page allocator,
   interrupt handling and time, then a scheduler and isolated processes.
3. Add virtual-machine drivers for storage, input, display and networking;
   define device ownership, DMA and failure recovery. These are VM devices,
   not drivers for physical Apple hardware.
4. Build persistent storage, IPC and service interfaces; extend them to the
   separate human/AI workspaces, attributed changes and AI-assisted merging.
5. Add the window manager, visual interaction, voice, MCP adapters and decision
   model integration described in the design documents. Each depends on lower
   layers and needs its own tests and performance budgets.
6. Separately study the Apple boot chain and implement or adapt real device
   drivers as platform evidence and licenses permit. Do not infer that a
   generic VM replicates the Mac's GPU, firmware, interrupts or memory devices.

The sequence is a roadmap, not a claim that these components are complete or
that it is the only viable OS architecture. It deliberately keeps native Mac
boot and compatibility claims separate from work in a virtual machine.

## VM devices and physical hardware

The owner wants the VM to use real peripherals where possible. A virtual machine
always runs on the physical host, but the interface exposed to the guest can
be emulated, paravirtualized through the host, or connected to an eligible
physical device. These are different validation targets:

| Resource | VM access in this research | What that proves |
|---|---|---|
| CPU | Current boot uses QEMU TCG to emulate an AArch64 CPU on the Mac. Hardware-assisted virtualization may be investigated separately. | CPU execution through this VM, not native M4 instruction or scheduler performance. |
| RAM | Guest RAM is allocated from host resources and mapped into the virtual machine. | Guest memory-management behavior on the virtual platform, not ownership of all physical RAM. |
| Serial, storage, display and network | QEMU `virt` can present virtual devices; current boot exposes only a serial console and has no disk or network. | A driver for a virtual device, not automatically a driver for the physical Mac device. |
| External USB | [QEMU documents host USB forwarding](https://www.qemu.org/docs/master/system/devices/usb) on certain hosts; support depends on build, permissions, device and guest driver. This setup has not been verified on our Mac. | If demonstrated for a specific device, exercises that device through the VM transport, not universal passthrough. |
| Internal Apple GPU, storage controller and other SoC devices | No direct device assignment has been demonstrated in this project. | A host-provided virtual interface, if available, cannot by itself prove a native M4 driver. |

Apple's Virtualization framework can expose virtual storage and other devices;
a guest may use a real disk as the backing for a **virtual block device**,
which is distinct from assigning the physical controller. See
[Apple's storage attachment documentation](https://developer.apple.com/documentation/virtualization/vzdiskblockdevicestoragedeviceattachment).
Passing a writable disk through can destroy data, so no existing disk is
attached to the test VM. Apple's newer USB passthrough APIs described for
[macOS 27](https://developer.apple.com/videos/play/wwdc2026/224/) must not be
assumed available on this macOS 15.1.1 host; Apple's earlier
[Sequoia description](https://developer.apple.com/forums/thread/757096)
describes virtual USB storage rather than arbitrary physical USB assignment.

Real device assignment requires a suitable hypervisor/host backend, permissions,
safe ownership, a compatible guest driver and device-specific verification.
Hardware that remains controlled by macOS or firmware may not be available for
direct assignment. The VM-only requirement applies to our tests; if a device
cannot be assigned to a VM, native driver behavior remains an open question
rather than something a simulated result can prove.

## Current code

The [ARM boot experiment](../experiments/arm_boot/README.md) is a minimal
freestanding Rust kernel for QEMU's generic AArch64 virtual machine. It has
booted in that VM and printed to a virtual serial console. It does not boot
on the M4 Pro or implement physical Mac drivers, a scheduler, memory management,
applications or a desktop. Its boot check is a functional result, not a
performance benchmark.

The [workspace core sketch](../experiments/system_core/README.md) is a small
Rust program running in user space. It models two copies and a three-way merge,
preserving conflicting edits for a future AI resolver. Its initial five tests
were run on the host before the VM-only requirement; future tests use a VM.
It does not implement a kernel, security boundary or actual AI resolution.

The [deep architecture](DEEP_ARCHITECTURE.md), [two workspaces](DUAL_WORKSPACES.md),
[AI control plane](AI_CONTROL_PLANE.md), and [voice assistant](VOICE_ASSISTANT.md)
define the larger goals. Their designs remain proposals unless an implementation
and its verification are explicitly recorded.
