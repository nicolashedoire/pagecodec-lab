# Existing Rust building blocks

The project aims to assemble existing components where they fit the intended
architecture, and implement missing parts. Entries here are candidates for
evaluation, not a claim that they are integrated, compatible with one another,
safe on a physical M4 Pro, or a complete operating system. A virtual device
driver and a physical Apple Silicon driver are different targets.

| Layer | Existing project | Potential role | Important limitation |
|---|---|---|
| AArch64 guest CPU | [aarch64-cpu](https://github.com/rust-embedded/aarch64-cpu) | Rust register and processor abstractions. | Low-level hardware behavior still requires target-specific correctness and unsafe-code review. |
| Virtual devices | [virtio-drivers](https://github.com/rcore-os/virtio-drivers) | Rust guest drivers for VirtIO devices. | Needs an OS-specific memory/transport integration; it is not a native Apple GPU, SSD or USB controller driver. |
| Device discovery | [dtoolkit](https://github.com/google/dtoolkit) | Parse flattened device trees in Rust. | QEMU FDT is not the same format as Apple's device tree; input validation and platform integration remain necessary. |
| OS reference | [Ferro](https://github.com/Rani367/ferro) | Experimental Rust AArch64 system with a desktop and multiple VirtIO drivers according to its source and README. | It is an early hobby OS, not a compatible macOS replacement. Its roadmap reports missing process/user isolation; its desktop has one pointer/focus. Its build includes separately licensed third-party code, so the repository's headline MIT license does not settle every component. |
| MCP | [Official MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) | Implement user-space MCP servers and clients. | A protocol library does not provide system permissions, kernel APIs, or drivers. |
| Shared documents | [Automerge](https://github.com/automerge/automerge) or [Yrs](https://github.com/y-crdt/y-crdt) | Concurrent, versioned collaborative data. | Convergence is not semantic conflict resolution by AI; formats and provenance still need design. |
| GUI | [Iced](https://github.com/iced-rs/iced), [wgpu](https://github.com/gfx-rs/wgpu) | Rust GUI and graphics abstractions. | They require a supported OS/windowing and graphics backend; they do not boot an OS or supply an M4 driver. |
| Audio | [CPAL](https://github.com/RustAudio/cpal) | Cross-platform application audio API. | It depends on an operating system and audio backend; it is not an audio driver for our new OS. |

Ferro's [roadmap](https://github.com/Rani367/ferro/blob/1df237c5909f1c45f5dec6139ade947f9407357c/ROADMAP.md#L16-L22)
and [desktop input code](https://github.com/Rani367/ferro/blob/1df237c5909f1c45f5dec6139ade947f9407357c/src/gui/input.rs#L6-L16)
illustrate why its current capabilities need adaptation for separate workspaces.
Our existing user-space workspace sketch uses Rust `std`, whereas both Ferro
and our bare-metal kernel use `no_std`; integrating them requires explicit
allocation, I/O and process boundaries, not just linking the two programs.

Before taking code, verify exact version, all transitive licenses, platform
assumptions, unsafe boundaries, ownership, build scripts and tests. Ferro, for
example, bundles additional code and assets with their own notices. Its source
also contains a [time-seeded SplitMix64 generator described for TLS](https://github.com/Rani367/ferro/blob/1df237c5909f1c45f5dec6139ade947f9407357c/src/net/rng.rs#L1-L68);
that must not be treated as secure entropy for TLS. We have not traced every
cryptographic call or established an exploit. We have not adopted
Ferro or tested it in a VM in this study.

The project's own first [AArch64 boot code](../experiments/arm_boot/README.md)
and [workspace core](../experiments/system_core/README.md) are separate starting
points. Integration must be explicit and verified inside VMs as requested; the
[system roadmap](RUST_SYSTEM.md) tracks which parts are actually present.
