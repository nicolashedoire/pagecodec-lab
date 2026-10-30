# MMU-backed graphical and storage experiment in QEMU

This research guest combines a copy of [`graph_unified`](../graph_unified/) with a narrow ARM64 identity map. It runs its graphical panels, synthetic QMP tablet clicks, MCP interactions and two-slot synthetic workspace snapshots with the EL1 MMU enabled in single-CPU QEMU `virt`. It uses the local [patched VirtIO driver](../virtio_fix/). The original experiments are unchanged.

## Mapping and transition

The guest requires EL1, masked interrupts, little-endian operation and instruction/data caches disabled at startup. It builds aligned 4 KiB translation tables, programs MAIR/TCR/TTBR0, invalidates EL1 translations and enables `SCTLR_EL1.M`; it checks that the MMU is enabled and caches remain disabled. TTBR1 walks are disabled. This setup is specific to the tested QEMU machine.

- The **linked guest image**, including its static 8 MiB heap, static 8 MiB DMA area, page tables and stack, is mapped VA=PA as normal non-cacheable memory. Text is EL1 read-only/executable with UXN; read-only data is EL1 read-only and XN; writable image data is EL1 read/write and XN. The remaining physical RAM outside the image is not mapped merely because QEMU provides it.
- One UART page and the four pages covering the 32 VirtIO MMIO slots are identity-mapped as Device-nGnRnE, EL1-only and XN. No IOMMU or general device isolation is configured.
- Boot assertions use ARM address-translation instructions to check identity for every page of the image, heap, DMA area and device ranges; they also check selected unmapped neighbors and sample EL1 text-write denial/data-write permission. EL0 read **and write** are probed only at selected data, image-end, UART and first/last VirtIO addresses. XN and Device attributes are checked by reading selected descriptors; the test does not execute from XN memory to provoke a fault. These samples are not exhaustive permission testing or proof of protected processes.
- Heap, DMA, page-table and stack placements are checked for bounds/alignment and relevant non-overlap before activation. Linker-symbol addresses are read through an opaque instruction before comparisons. `mmu_status` reports the result of these boot assertions after initialization; it is not continuous monitoring or an attestation.

The QEMU HAL uses the identity map to treat a guest virtual address as a guest physical address. `dma_alloc` and `share` check bounds and translate **each page of the full shared range** before returning it; MMIO conversion verifies the mapped range. Shared writable ranges may include the guest heap, stack **and page tables**. The checks do not provide DMA isolation, ownership, direction enforcement, confidentiality or a hardware IOMMU. Correct pointer provenance, length, lifetime and synchronization remain unsafe caller obligations. The one-shot allocator and no-op release paths do not reclaim memory. Disabled caches and successful QEMU runs do not prove coherence or correct DMA barriers on real hardware.

## QEMU evidence and limits

The [modern-MMIO evidence](evidence/graph-mmu-evidence.json) and [legacy-MMIO evidence](evidence/graph-mmu-legacy-evidence.json) each record eight guest instances, their MCP `mmu_status` results, synthetic QMP/MCP I/O, slot hashes and framebuffer hashes. Each run verifies:

1. A synthetic QMP human click and MCP assistant click change separate in-guest logical focus/counters and produce framebuffer changes. Buttons do **not** edit documents.
2. Synthetic MCP document edits create two copies and a conflict, then commit a snapshot. A fresh VM restores the copies and recomputes the conflict/origins without changing its pointer/focus state.
3. The host terminates QEMU after a deliberately staged, uncommitted write; the next VM selects the earlier valid snapshot. Further runs test a newer commit, fallback after corrupting that newer slot, rejection when both slots are corrupt, and rejection of an uncommitted first-ever snapshot without overwriting it.

The local VirtIO patch checks modern feature-negotiation acceptance, while preserving the prior legacy behavior; these runs use accepting QEMU devices, not a GPU/tablet/block device that rejects negotiation. The runner creates every 1 MiB image exclusively in a private temporary directory and accepts no existing disk or physical device. QMP input is synthetic. The drawn panels are not real applications, windows or protected processes; focus is a guest-local state. Actor labels and MCP access are unauthenticated, pointer and document registries remain distinct, and original document event history is not persisted.

An MCP click changes logical state before GPU flush; on flush failure the guest returns an error and halts without rollback. The stage failpoint returns before the host sends SIGTERM; it does not interrupt an individual write or simulate power failure. `mode=commit` serializes the current document anew rather than finalizing a previous stage. A successful driver `flush()` return does not prove FLUSH was negotiated or reached a device. The snapshots use a non-cryptographic checksum and have no authentication, atomicity, crash durability, device-failure recovery or concurrent-writer guarantee. Missing/read-only devices, malformed inputs beyond the recorded cases, IRQs/preemption, multicore, real hardware and general-purpose address spaces are not tested. No Apple hardware or performance claim follows.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/graph-mmu-demo \
  --output-dir /path/to/empty-output
# Optional: repeat with --transport legacy and a different empty directory
```

The host harness requires an empty output directory, starts isolated QEMU instances without network or host directory sharing, sends synthetic MCP/QMP requests and inspects only images it creates. It records the guest hash, QEMU version, status reports, protocol traces, framebuffer and slot hashes. Do not rebuild the guest during a run; the binary hash is checked before and after.
