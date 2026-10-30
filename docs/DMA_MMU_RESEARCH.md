# DMA and MMU requirements for a common QEMU guest

[`graph_mmu`](../experiments/graph_mmu/README.md) now demonstrates the graphical and synthetic storage workflow under an EL1 MMU in QEMU using a cache-off identity map. Its [modern](../experiments/graph_mmu/evidence/graph-mmu-evidence.json) and [legacy](../experiments/graph_mmu/evidence/graph-mmu-legacy-evidence.json) traces cover the selected virtual devices. This remains separate from [`kernel_integrated`](../experiments/kernel_integrated/src/main.rs), which demonstrates EL0 exceptions and an EL1 timer, and [`el0_preemption`](../experiments/el0_preemption/README.md), which tests two EL0 loops in one address space. Those guests are not yet one system.

The older [`graph_unified`](../experiments/graph_unified/src/main.rs) HAL assumes a single CPU, MMU and caches disabled, and identical guest virtual and physical addresses. `graph_mmu` instead verifies identity translations page by page for its shared ranges; caches remain disabled. Its permission probes are sampled, not an exhaustive fault test.

## Address ranges and page attributes

The graphical guest reserves an 8 MiB heap and an 8 MiB page-aligned DMA arena, in addition to its image, stack and device buffers. `kernel_integrated` maps its bounded RAM image using one 2 MiB L3 region; `graph_mmu` instead uses enough L3 tables for its image, and checks the image, heap, DMA and device translations page by page. A combined kernel needs a shared linker layout, checked range endpoints and non-overlapping reservations. The [frame allocator model](../experiments/memory_core/README.md) does not discover RAM or establish these mappings.

The graphical guest scans 32 VirtIO-MMIO slots at `0x0a000000` with a `0x200` stride, spanning 16 KiB, or four 4 KiB pages. The common QEMU configuration must map the required pages as privileged device memory with execute-never permissions. UART and GIC are separate regions. Heap, DMA and ordinary RAM need Normal memory attributes appropriate to the selected QEMU configuration and execute-never where they do not contain code. The existing MMU fixture uses Device-nGnRnE for device registers and Normal non-cacheable memory for RAM; adopting those attributes for a larger guest still requires reviewing the DMA contract rather than assuming correctness. Prevent EL0 access to device registers and kernel buffers.

Identity-translation and permission probes should cover both endpoints of every mapped range, any page boundaries crossed, representative holes, and pages immediately outside the mappings. Translation probes and descriptor inspection establish properties of the selected mappings; they do not independently prove every DMA operation, device ordering rule or hardware isolation property.

## HAL and buffer ownership

In `graph_mmu`, `dma_alloc` and `share` check guest bounds and identity translation for **every page of the complete shared range**, then cast its virtual address as the guest physical address. The allowed writable image range includes heap, stack and page tables. These checks do not enforce ownership or direction and provide no IOMMU or DMA isolation; `unsafe` caller obligations remain. A more general guest needs an appropriate translation strategy and explicit lifetime contract for every shared range.

Device-visible memory must stay allocated, pinned and valid until the corresponding device operation completes and the driver calls `unshare` or releases the DMA allocation. Define access direction, prevent conflicting CPU/device access and Rust mutable aliases, and ensure that no allocator or page-table update can recycle the underlying pages prematurely. The current `dma_dealloc` and `unshare` do not reclaim memory, and the graphical heap is a finite non-reclaiming bump allocator. This is acceptable only as a bounded fixture with exhaustion handling, not as a general long-lived allocator.

For all `unsafe` implementations, document and enforce page alignment, nonzero and bounded allocation sizes, checked arithmetic, permitted address ranges, stable raw-pointer provenance, valid lifetimes and no overlapping mutable references. Device registers need volatile access; ordinary volatile operations alone do not establish safe DMA synchronization. Any larger physical-frame allocator must reserve image, page tables, stack and DMA memory before giving frames to another owner.

## Ordering and cache assumptions

The local [VirtIO driver](../experiments/virtio_fix/vendor/virtio-drivers/) contains queue fences, but their presence by itself is not a proof that the complete HAL, device and mapping configuration obey every required ordering and coherency contract. Review descriptor publication, device notification, completion observation and buffer access against the VirtIO transport and target architecture. Check the required compiler and hardware barriers, memory attributes, sharing direction and cache maintenance for the selected configuration. Cache-disabled and Normal non-cacheable configurations simplify parts of the problem but do not remove the need to establish ordering and ownership. Do not enable caches, change mapping attributes or claim general coherence without separate verification.

## VM-only integration checks

1. Verify the linker layout and each mapping's full range, permissions and neighboring holes; demonstrate that EL0 cannot read or write the device and kernel regions.
2. Exercise heap and DMA allocation, exhaustion, alignment, overflow and disjoint ranges with synthetic data. Verify any temporary shared buffer remains valid until device completion, with no premature reuse.
3. Under the selected MMU and cache configuration, test actual VirtIO GPU, tablet and block operations for both modern and legacy MMIO. Compare synthetic device outputs, framebuffer changes and block contents with the expected data.
4. Repeat save, stop and reload on a runner-created temporary disk and retain guest hash, QEMU version, requests and results. A controlled QEMU stop does not establish power-loss durability or physical-device atomicity.
5. Add negative scenarios for missing or rejected devices, invalid ranges, exhausted buffers and failed operations; ensure no unintended guest memory region changes.

These tests establish only the tested virtual-platform behavior. They do not prove general DMA isolation, confidential access, native Apple Silicon drivers, cache behavior on Apple hardware, or readiness to boot or install an operating system on a Mac.
