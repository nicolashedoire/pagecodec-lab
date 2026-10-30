# ARM64 preemptive scheduling: research and integration constraints

Status: **design proposal, not an implemented preemptive scheduler**. The current experiments run separately in single-CPU QEMU ARM guests. Nothing here establishes native Apple Silicon support, hard real-time behavior, protected processes or full architectural context preservation.

## Existing evidence

- [`cooperative_tasks`](../experiments/cooperative_tasks/src/main.rs) saves SP and x19–x30 when a task voluntarily calls the switch path. Its test checks x19–x29, SP, selected stack values and the execution order across two task stacks. Return through x30 is exercised by control flow, not by an independent arbitrary-value test. This is a cooperative call boundary, not an interrupt context.
- [`arm_timer`](../experiments/arm_timer/src/main.rs) receives a single QEMU virtual-timer IRQ via GICv2 PPI 27. Its handler saves x0–x30 on the current stack, disables the timer, writes EOI and returns with `eret`. The probe checks the general registers, SP and NZCV. Crucially, the handler accepts only one known interrupted instruction and **replaces** its `ELR_EL1` with a designated continuation. This is a controlled IRQ experiment, not transparent return to an arbitrary interrupted instruction or task switching.
- Neither experiment currently stores a complete preemptible execution state per task, switches tasks from an IRQ, or demonstrates progress of two CPU-bound tasks without voluntary yields. The exact QEMU machine and limitations are in each experiment's own runner and evidence.

## Architectural facts and assumptions

On AArch64 exception return, `eret` restores state from `SPSR_ELx` and the program counter from `ELR_ELx`; those registers therefore cannot be treated as ordinary process-global scratch if exceptions can nest or a different task is selected. [Arm exception model](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/Exception%20model.pdf).

A normal procedure call and an asynchronous interruption have different preservation requirements. AAPCS64 specifies call-preserved registers and stack alignment at call boundaries; it does not turn caller-saved registers into disposable state when an interrupt strikes between arbitrary instructions. The ABI also specifies FP/SIMD preservation rules for calls. [Arm AAPCS64](https://github.com/ARM-software/abi-aa/blob/main/aapcs64/aapcs64.rst).

These facts do not by themselves specify the operating system's complete task state. A design must also account for its execution level, stack selection, architectural extensions and permitted FP/SIMD use, and interrupt-controller configuration. Rust pointer and `volatile` operations do not themselves establish synchronization, exclusive ownership or memory safety across a preemption boundary.

## Proposed first integration

This is a **single-CPU, QEMU-only** initial design to evaluate, not verified code:

1. Define a per-task exception frame containing at least x0–x30, the interrupted stack/frame location, `ELR_EL1` and `SPSR_EL1`, with a 16-byte-aligned valid stack and unambiguous frame ownership. Record them before allowing another exception to overwrite the exception registers. Define entry from the selected EL and stack mode explicitly.
2. Create a consistent initial frame and trampoline for a new task, and resume a suspended task using the same well-defined restore path and `eret`. Do not reinterpret the cooperative SP/x19–x30 record as a full IRQ frame. The interrupted PC must be restored as saved, not replaced with the existing timer probe's hard-coded continuation.
3. For an expected timer IRQ, acknowledge through the configured interrupt controller, deassert or mask the source and perform the required EOI/deactivation for that controller configuration before switching away. Under a defined IRQ/preemption masking discipline, record the outgoing frame, select a runnable task using preallocated state, arrange the next timer event, restore the selected frame and return. Make the order, ownership and no-runnable-task path explicit; handle an unexpected or spurious interrupt without silently treating it as a timer.
4. Do not allocate, perform blocking I/O, sleep or acquire a blocking lock in the interrupt path. Define which short critical sections forbid preemption and ensure interrupt code cannot re-enter mutable scheduler or allocator state. Keep task, frame and stack lifetimes valid while suspended. Distinguish volatile MMIO from synchronization and Rust aliasing guarantees. [Linux kernel documentation on preemption and locking](https://docs.kernel.org/locking/preempt-locking.html) is a useful example of why interrupt/preemption rules need explicit discipline, not a prescription to copy Linux's scheduler.
5. Initially prohibit FP/SIMD use by the guest configuration and implementation, and verify that restriction in the generated code and test environment before claiming it. Supporting FP/SIMD or other enabled architectural state requires its own ownership, save/restore and tests; the current soft-float target alone does not prove a universal context implementation.

An alternative is to acknowledge the timer and set a reschedule request, then switch at a defined safe point outside hard IRQ context. That can simplify locking, but it only demonstrates preemption if CPU-bound tasks actually reach the mechanism without voluntarily yielding and no task can indefinitely defer it. State exactly which path is implemented and tested.

## Counterexamples and VM validation

A successful demonstration should record synthetic inputs, IRQ decisions, selected task IDs and results without claiming measured performance:

- Run two CPU-bound tasks that never call a voluntary yield; verify each progresses after multiple timer interrupts and that switching actually came from the timer path. A loop that explicitly calls a scheduler is a counterexample to the claim of timer-driven preemption.
- Interrupt at multiple non-fixed PCs. Resume the saved interrupted instruction and verify x0–x30, SP and NZCV with task-specific sentinels, including caller-saved registers. Verify task-local stack values and actual frame/stack ownership across repeated switches, not just one return to the controlled probe.
- Check initial task entry, repeated scheduling of the same task, zero and one runnable task, task completion, and no accidental reuse of a live frame or stack. Test stack alignment and guards/overflow handling before treating task stacks as protected.
- Exercise timer rearming, interrupt acknowledgement, source deassertion and EOI without duplicate dispatch or lost runnable tasks; account for nested interrupts or explicitly keep them masked. Inject unexpected IDs and spurious interrupts and verify the declared response. Test interrupt arrival near frame creation, restore and protected critical sections.
- Confirm a rejected or deferred switch cannot corrupt the ready queue, a task's state or the journal; verify IRQ/preemption restoration and allocator ownership around critical regions. A shared mutable Rust reference held across an unprotected interrupt that mutates the same object is a counterexample to a safe integration.
- If FP/SIMD or other extensions are enabled, give tasks distinct live register and control-state values and verify isolation over repeated switches. A build/run that only uses integer code does not establish that result.

Tests must run inside the VM and log what actually happened. Early results on one QEMU version with synthetic tasks cannot establish fairness under general workloads, hard real-time latency, multicore correctness, native DMA behavior or Apple hardware compatibility. Interrupt nesting, exceptions other than the tested IRQ, MMU/address-space changes, process isolation, fault recovery and real applications require separate designs and tests.
