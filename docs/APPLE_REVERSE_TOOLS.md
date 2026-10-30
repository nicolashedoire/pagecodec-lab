# Apple Silicon reverse-engineering tools

We use public XNU source, REA and Ghidra to examine a local copy of the kernel.
On the research Mac, we also tested [IPSW](https://github.com/blacktop/ipsw)
release 3.1.739 in a workspace directory, after checking the release archive
against the publisher's SHA-256 checksum. No system files were changed. IPSW
identified the existing extracted ARM64e kernel collection as a Mach-O FILESET
and listed 389 kext entries from that image. This inventory describes the
image, not a live list of running or loaded components. It complements REA and
Ghidra when inspecting Apple kernel collections, Mach-O binaries and shared
libraries; it does not expose all runtime behavior or replace source analysis.
The checksum establishes agreement with the downloaded publisher checksum,
not an independent security audit of the release.

[Fractal](https://github.com/jprx/fractal) is a separate research operating
system for studying microarchitecture and supports a QEMU target. Its documented
physical M4 support is restricted to Mac mini and macOS 15.1 exactly, so the
research Mac running 15.1.1 is not established as compatible. We have not
built or booted it. A VM can help functional experiments, but cannot by itself
reproduce the physical processor's caches and scheduling behavior.

[Asahi Linux's m1n1](https://github.com/AsahiLinux/m1n1) is another low-level
Apple Silicon experimentation platform. Its hardware boot path and compatibility
require separate evaluation; it has not been installed or booted here.
