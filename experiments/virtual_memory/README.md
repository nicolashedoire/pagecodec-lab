# Bounded EL1 address translation in QEMU

This standalone Rust `no_std` guest sets up and enables a small AArch64 EL1 stage-1 translation regime **inside QEMU**. It is a controlled VM experiment, not an OS process-isolation system or a native Apple Silicon memory manager. No host memory mappings or host devices are changed.

The guest requires one virtual CPU at EL1, with IRQs masked and the MMU, data cache, instruction cache and big-endian translation-table mode initially disabled. Its linker lays out page-aligned text, read-only data, writable data/BSS, static page tables and a guest stack inside the first 2 MiB of QEMU's RAM region. Before enabling translation, the guest checks the image, stack and six distinct 4 KiB table addresses lie within that expected region. It builds four-level 4 KiB translation tables with invalid entries by default. Only the guest image and stack pages and the single QEMU PL011 UART page are identity mapped; no broad RAM or MMIO block is mapped.

For this narrow configuration, the guest chooses a 48-bit TTBR0 virtual range, 32-bit output addresses, 4 KiB granules and disabled TTBR1 walks. `MAIR_EL1` describes normal non-cacheable memory and Device-nGnRnE; table walks are non-cacheable. Text is mapped read-only for EL1 and executable, read-only data is mapped read-only and execute-never, writable data/stack and UART are marked execute-never, and the page permissions do not grant EL0 access. It installs the table base and control registers, issues barriers and TLB invalidation, enables the MMU, and continues execution on the identity mapping.

After activation the guest writes and reads a synthetic value in its own mapped data, uses `AT S1E1R` and `PAR_EL1` to check identity translations for the image, data, stack and UART, and confirms that selected pages immediately outside the image and UART do not translate. `AT S1E1W` checks that a text write is rejected and a data write is allowed; `AT S1E0R` checks that selected data and UART reads are rejected at EL0. It checks execute-never bits in selected page-table descriptors, **without attempting execution from those pages**. These translation queries do not deliberately dereference unmapped pages or install exception recovery.

The recorded evidence establishes behavior for this single QEMU configuration and selected addresses. It does not prove every permission or memory attribute, real address-fault handling, complete hardware discovery, ASIDs, multiple address spaces, kernel/user isolation, concurrency, shootdowns, swapping, DMA correctness, performance or Apple hardware support. Text execute-never/permissions are not a substitute for an independent audit; the guest has no process or task model and its whole kernel image is trusted in one address space. Interrupts remain masked and no timer or device IRQ runs here. The UART address and RAM location are QEMU-specific, not Mac physical addresses.

Arm's [memory-management guide](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/LearnTheArchitecture-MemoryManagement-101811_0100_00_en.pdf) explains `TTBR0_EL1`, `TCR_EL1`, `MAIR_EL1` and `SCTLR_EL1`. This demo implements only the stated configuration, not a general Arm MMU implementation.

## Reproduce in QEMU

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/virtual-memory-demo \
  --output-dir /path/to/output
```

The host compiles the guest, launches a single QEMU VM with no networking, disks or shared host directories, and collects its synthetic serial log. All MMU configuration, reads/writes and address-translation queries occur inside the VM. The runner checks the guest hash before and after the run. See the [recorded QEMU evidence](evidence/virtual-memory-evidence.json). Do not rebuild the ELF while the runner is executing.
