# Integrating the QEMU experiments into a kernel

This document describes an integration plan and bounded integration steps, not a complete kernel. The [EL0 guest](../experiments/user_mode/README.md), [preemptive blocking guest](../experiments/preemptive_blocking/README.md), [graphical interaction guest](../experiments/graph_interaction/README.md), [frame allocator model](../experiments/memory_core/README.md) and [storage recovery guest](../experiments/graph_crash_recovery/README.md) each provide evidence for their own scenarios. Their separate successes do not establish that all these mechanisms work together.

The new [`kernel_integrated` guest](../experiments/kernel_integrated/) combines MMU setup, a controlled EL0 permission fault and SVC return, and a virtual-timer IRQ through one EL1 vector table. The timer is armed **after** the final EL0 SVC and interrupts an EL1 wait loop: this is not EL0 preemption or scheduler integration. The separate [`graph_unified` guest](../experiments/graph_unified/README.md) combines synthetic graphical interactions and two-slot document snapshots under QEMU, using its own runtime and MMU-disabled HAL. These are separate binaries; neither is a complete operating system.

## Where the prototypes meet

| Concern | Current evidence | Work needed for a common guest |
| --- | --- | --- |
| Entry and runtime | Each standalone guest has its own boot code, linker script, console and failure handling. | Define one entry point, linker layout, initialization sequence and serial/error interface. Keep the standalone experiments for regression. |
| Memory | [`user_mode`](../experiments/user_mode/src/main.rs) installs a fixed identity-mapped page table for its image and UART. [`memory_core`](../experiments/memory_core/README.md) tracks a caller-provided synthetic pool; it does not discover or map RAM. | Establish a trusted QEMU RAM and reservation map, account for the kernel, stacks, tables and device buffers, then connect frame ownership and page mappings. A returned frame address is not automatically safe to dereference. |
| Interrupts and exceptions | [`kernel_integrated`](../experiments/kernel_integrated/src/main.rs) uses one table for lower-EL synchronous exceptions and a subsequent EL1 GICv2 timer IRQ; [`preemptive_blocking`](../experiments/preemptive_blocking/src/main.rs) remains a separate guest. | Extend context and dispatch for EL0 preemption and scheduler integration; verify state beyond the selected fixture. |
| Scheduling | The preemptive guest switches two fixed EL1 loops using a scripted timer scenario. | Define task lifecycle, a bounded ready queue and synchronization contracts before scheduling real EL0 tasks. Add genuine block/wake events and idle behavior; do not treat scripted timer events as an application API. |
| Heap and DMA | [`graph_interaction`](../experiments/graph_interaction/src/main.rs) uses a finite non-reclaiming bump heap and a static DMA arena. | Provide a bounded, correctly synchronized heap and explicit buffer ownership, failure behavior and eventual reclamation. No heap allocation or logging in IRQ context. |
| VirtIO and graphics | The graphical guest has its own local device setup and [patched VirtIO dependency](../experiments/virtio_fix/). | Adapt the transport and HAL to the common mappings and IRQ design, then test GPU/input operation in the integrated guest. |
| Documents and persistence | [`graph_unified`](../experiments/graph_unified/README.md) combines interaction and two-slot snapshots using a temporary virtual block device, but retains its own runtime. | Integrate those services into the shared kernel and test interrupted writes; the current stage/SIGTERM fixture does not prove atomicity. |

### MMU and DMA mismatch

The graphical guest's `VmHal` explicitly assumes a single CPU with **MMU and caches disabled**, and identical guest virtual and physical addresses. The original EL0 demonstration enables the MMU and maps its bounded image and UART; `kernel_integrated` additionally maps GIC pages, but not VirtIO MMIO or DMA. Combining the graphics code unchanged therefore fails to establish the mapping, lifetime, coherency and ownership assumptions needed by DMA. A new HAL must translate addresses according to the actual guest mapping, bound and pin buffers for the duration of device access, use appropriate memory attributes and barriers, and handle direction and device completion. Validate those choices for the chosen virtual platform; successful QEMU tests do not prove a native Apple DMA implementation.

### Rust and exception safety

The shared implementation must make each `unsafe` boundary explicit: valid physical and virtual ranges, checked arithmetic, alignment, unique mutable ownership, volatile MMIO, DMA lifetime, stack bounds and synchronization. Static mutable state and raw-pointer access in isolated single-CPU demos cannot simply be reused as a concurrency model. Preemption must not observe partially initialized data; IRQ paths must not allocate or block. Page-table changes require architecture-correct translation maintenance and barriers. Context switching must preserve all state the supported tasks actually use; the present demos do not save general FP/SIMD context. Unrecognized exceptions need a defined failure path rather than silently continuing.

## Ordered milestones

1. **Common boot:** one QEMU `virt` guest, one linker and entry point, UART and a recorded smoke test. Define its CPU count and virtual platform configuration.
2. **Memory foundations:** establish RAM bounds and reserved ranges, integrate frame metadata without overlapping ownership, map image, stacks, UART, GIC, VirtIO MMIO and device buffers with appropriate attributes, and test valid and invalid mappings. Define zeroing and allocation-failure behavior.
3. **Unified exceptions:** `kernel_integrated` demonstrates one vector table, a controlled EL0 synchronous exception and a subsequent EL1 virtual-timer IRQ with acknowledgement and return. Extend this to interrupts during user execution and other exception paths; keep interrupts masked until setup is complete.
4. **Task and allocation contracts:** implement bounded task states and heap ownership; test queue integrity, exhaustion, blocking and waking without allocation or serial output inside IRQ handlers. Add stack bounds and explicit context preservation tests.
5. **EL0 plus scheduling:** run a synthetic user task, exercise syscall and denied-access paths, then preempt and resume it using the common context and address-space contracts. Test failure cases and state preservation.
6. **Devices and services:** adapt VirtIO DMA/MMIO to the active memory configuration, then attach graphical input and MCP, followed by temporary virtual storage and document recovery. Verify each integrated end-to-end path and its negative cases inside QEMU.

Each stage should record the exact guest image, virtual platform, inputs, outputs and limitations. Benchmarks are meaningful only after functional integration. Nothing in this plan proves a boot path, driver compatibility or installation readiness on a physical Mac; those require separate platform-specific engineering and verification.
