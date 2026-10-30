# Running graphical services and EL0 tasks concurrently in QEMU

This is an integration proposal. The current [`system_vm`](../experiments/system_vm/README.md) runs two fixed EL0 loops for eight timer IRQs, stops that probe, masks interrupts and then transfers to its graphical/MCP loop. Its phases are sequential: no graphical request is processed while those EL0 loops run. One CPU can interleave execution, but this is not multicore parallelism.

## The current ownership boundary

[`graph_main`](../experiments/system_vm/src/main.rs) owns the router, document copies, interaction state, pending MCP replies and VirtIO GPU, tablet and block drivers on its EL1 stack. Its event loop polls the devices and UART, handles one request, updates state and may flush the framebuffer or block device. This is a single-owner, non-reentrant arrangement. The current EL0 finish path disables the timer/GIC and [`resume_graphics`](../experiments/system_vm/src/el0.rs) resets the EL1 stack before starting this loop. Resetting that stack while an existing graphical task is suspended would destroy its local state and return context.

The existing EL0 IRQ path saves a 272-byte integer frame including `SP_EL0`, `ELR_EL1` and `SPSR_EL1`, but it handles the lower-EL IRQ fixture, not preemption of a live graphical EL1 task. Its two EL0 loops share an address space and do not run applications. It does not establish general task scheduling, SVC handling, nested IRQ recovery or FP/SIMD and extension-state preservation.

## Candidate single-CPU design

One incremental candidate is a persistent EL1 service task, with its own stack and saved execution context, and fixed EL0 test tasks. One vector table would dispatch interrupts taken from both EL1 SPx and EL0. The scheduler would save the interrupted task's actual frame and stack pointer, choose a runnable context, and restore its mode, PC, status and registers before returning. An EL1 continuation needs a persistent, bounded kernel stack; it cannot be recreated by jumping back to `graph_main`. User tasks need their own saved user and exception context. If address-space roots change, all kernel code, vectors, driver mappings and active kernel stacks required during the transition must remain valid under each root.

This is a design to implement and verify; the separate [`address_spaces`](../experiments/address_spaces/README.md) experiment does not yet make it part of `system_vm`. A complete context must cover all architectural state actually used by the selected build, including extension state if enabled. The exact context format, task lifecycle and failure behavior must be specified before describing the result as a general scheduler.

## Interrupts, Rust state and device access

Keep the interrupt handler bounded: acknowledge and rearm the timer/GIC, save or restore context, and update a preallocated scheduling journal or queue. It should not invoke MCP parsing, allocate, print, draw or call a VirtIO driver. Use short, well-defined critical sections for scheduler metadata and interrupt mask transitions; do not hold an interrupt mask across a blocking framebuffer or disk flush. Unexpected or nested exceptions need explicit behavior.

Only the graphical service should own and mutate its non-reentrant driver instances, framebuffer, router and documents. An EL1 task can be preempted while a Rust borrow or mutable slice exists only if no interrupt or other task accesses the same object until that task resumes. Future cross-task requests should pass through a bounded ownership-safe queue or another specified interface; atomics on the bump allocator do not make `serde_json`, the router, the drivers or the framebuffer reentrant. DMA buffers must stay mapped and valid until device completion, with direction and synchronization requirements established separately; see [DMA and MMU research](DMA_MMU_RESEARCH.md).

The current heap and DMA arenas are finite and do not reclaim allocations. A concurrent demonstration must preserve explicit capacity and failure behavior, avoid allocation in IRQ context and avoid assuming these one-shot allocators are suitable for a long-running service. Storage snapshots and MCP actor labels retain the limitations documented in [workspace persistence research](WORKSPACE_PERSISTENCE_RESEARCH.md).

## Proposed VM-only tests

1. Demonstrate a timer IRQ actually interrupting a PC inside the graphical EL1 task, and record the interrupted mode, stack bounds and selected next task. Also observe an IRQ from EL0. Validate integer frames and relevant flags on both paths.
2. Retain and check a known local stack value and service state across repeated preemptions, including around a request and device operation. Verify that no handler accesses a suspended task's borrowed Rust state.
3. Exercise synthetic QMP input and MCP requests while EL0 counters make progress, checking the request results, graphical state, framebuffer changes and selected task IDs. The result demonstrates interleaving on one CPU, not simultaneous execution.
4. Add a temporary virtual block device and test snapshot/reload while scheduling continues. Keep the existing limitations on flush semantics, stage/SIGTERM and crash durability; detect errors without claiming a successful save.
5. Test bounded queue exhaustion and invalid transitions; verify IRQ paths do not allocate or emit serial output, stacks remain in bounds, and unexpected vectors fail explicitly. Capture synthetic I/O and loss information with a bounded log.
6. Test any added address-space changes with the relevant translation and TLB rules, and validate actual cross-task access failures where claimed. Do not infer IOMMU protection, native Apple compatibility, timing guarantees or full process isolation from these QEMU runs.
