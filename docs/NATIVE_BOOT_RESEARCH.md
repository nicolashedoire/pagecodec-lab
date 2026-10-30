# Native Apple Silicon boot: research and remaining work

Status: research, **not an installation procedure**. Reviewed 2026-10-30. The project currently boots an experimental Rust kernel on the QEMU ARM `virt` machine; it does not boot or operate as an installed OS on the target M4 Pro Mac. The VM presents virtual devices such as VirtIO and PL011, not the Mac's physical controllers. Passing the VM tests therefore cannot establish native boot, device support, macOS application compatibility or performance.

The project goal is eventually to run on the actual Mac while retaining an operational recovery path and protecting existing data. No disk partition, firmware, startup setting or security policy has been changed as part of this research. This page describes a possible direction and open requirements; it does not authorize treating the draft boot chain as proven on this machine.

## Evidence from Apple and Asahi

* [Apple's boot security description](https://support.apple.com/en-ca/guide/security/secac71d5623/web) describes a chain beginning in Boot ROM, followed by LLB and iBoot. LLB loads a machine-specific LocalPolicy signed by the Secure Enclave; the selected security configuration affects subsequent verification. Apple documents security levels and a mode for user-built XNU kernels, but that description alone is **not** a validated procedure for booting this project.
* [Asahi's open OS interoperability draft](https://asahilinux.org/docs/platform/open-os-interop/) proposes an Apple-recognized APFS stub OS, m1n1, device trees and a later loader, enabling a third-party OS to coexist with other installations. It is a draft platform design, not evidence that our kernel or this M4 Pro can use the chain today. Availability and exact behavior depend on machine, firmware, selected tools and versions.
* At the review date, [Asahi's M4 series support matrix](https://asahilinux.org/docs/platform/feature-support/m4/) lists M4 Pro/Max NVMe and AICv3 as work in progress; GPU, display controller, DART, PCIe and USB are among entries marked TBA. For the M4 Pro/Max models listed in the table, the installer is marked unavailable and the device tree is work in progress. WIP and TBA have the definitions on that page and may change. These are Asahi's project statuses, not a universal claim that no independent work exists.
* [Asahi's partitioning documentation](https://asahilinux.org/docs/sw/partitioning-cheatsheet/) explains that Apple Silicon uses special APFS partitions and recovery components. Preserving an existing macOS installation and its recovery resources is a design requirement, not a substitute for a verified backup.

## Candidate chain, not implemented

```text
Apple Boot ROM → LLB + LocalPolicy → iBoot
                                  ↓
                 dedicated Apple-recognized OS stub
                                  ↓
               m1n1 → device description → later loader
                                  ↓
          native ARM kernel → native drivers → system services
```

This is an adaptation of the interoperability draft to our goal; the exact chain, boot artifacts and placement are unresolved for the target Mac. The draft proposes a separate stub, an EFI partition with m1n1 stage 2, device trees and U-Boot, and OS-specific partitions. It does **not** demonstrate this kernel on M4 Pro. Booting from external storage would not by itself eliminate Apple's boot and internal firmware dependencies. It is premature to provide or execute partitioning, signing, installation or policy-change commands.

## What must be built and verified

1. **System completeness in the VM:** reproducible boots and regression tests, memory and interrupt management, scheduling and isolation, persistence, input, graphics, error handling and recovery. MCP, multi-actor workspaces and the merge service must work as actual components, with clear boundaries and auditability. A graphical mockup is not a complete OS.
2. **Target identification and provenance:** record the actual model, board and firmware versions; pin the sources, licenses and toolchain revisions used for the native port. Recheck support tables at the time of any future decision.
3. **Native boot handoff:** demonstrate a supported, repeatable handoff for the exact machine and firmware; identify the required Apple-provided artifacts, relevant security policy and a working route back to macOS. The need and exact method for any policy change remain to be established. Do not treat a generic ARM QEMU boot as this proof.
4. **Hardware foundations:** establish the actual device description, address mapping, interrupt controller, clocks, power management, memory protection and DMA/IOMMU requirements. Implement and test the necessary interfaces; Rust is the preferred implementation language where practical, but correctness and hardware contracts need evidence.
5. **Usable devices:** develop or reuse, subject to license and compatibility review, the necessary storage, display, input and USB drivers, followed by graphics acceleration, network, audio, power and additional devices. VirtIO guest drivers are useful for VM development and do not drive the corresponding integrated Apple devices by themselves.
6. **Reliability and rollback:** verify test results and logs, test data integrity and failures, preserve the existing boot environment, establish a restorable backup and a documented path to recovery. Native installation becomes a candidate only after the exact approach and dependencies have been independently checked.

### Recovery distinctions

Apple documents [startup and macOS Recovery](https://support.apple.com/guide/mac-help/macos-recovery-a-mac-apple-silicon-mchl82829c17/mac) and [firmware revive and restore](https://support.apple.com/en-ie/108900). Firmware **revive** is documented as not erasing the Mac, while **restore** erases it; both require another appropriately prepared Mac and a suitable cable. Recovery options do not guarantee that every failure can be repaired without data loss. A separately verified, restorable backup remains necessary before experimenting with boot or disk configuration.

## Reuse, licensing and research boundaries

[m1n1](https://github.com/AsahiLinux/m1n1) is an Apple Silicon bootloader and experimentation project. Its repository contains more than one license and third-party materials; check the exact revision, components and intended distribution before reuse. Firmware redistribution rights and the route for obtaining firmware must also be checked for each artifact; Asahi's interoperability document describes local firmware provisioning. No bundled Apple firmware is added to this repository by this research.

Asahi's [generative AI policy](https://asahilinux.org/llm-policy/) restricts AI-assisted material contributions and emphasizes clean-room provenance. Respect that policy before proposing any upstream contribution. Source availability, reverse-engineering records and license terms need independent review; research here is not a conclusion about the legal status of any proposed reuse.
