# EL0 task slices interleaved with a graphical QEMU service

This experiment extends a copy of [`system_vm`](../system_vm/) in one ARM64 QEMU image. After the initial fixed EL0 preemption probe, it continues to run two synthetic CPU-bound EL0 assembly loops while servicing graphical input, MCP and snapshot operations on a single EL1 service stack. **The service yields voluntarily at outer-loop boundaries; it is not preempted in the middle of a Rust, allocator or VirtIO operation.** Thus the result is single-CPU interleaving, not multicore parallelism, arbitrary kernel preemption or evidence that EL0 runs during an individual blocking request.

The service yields after a completed UART request or input event, or after 20,000 outer-loop polls. The poll budget is an iteration count, not a time guarantee: an incomplete UART request can be interrupted between polling iterations, and a long-running handler or blocked device operation can delay the next yield indefinitely. The two EL0 loops share an address space and access the same user pages; they are not isolated processes or applications. The graphical panels remain synthetic framebuffer controls, not real windows. Neither actor labels nor MCP access are authenticated.

## Context switching and scope

At each voluntary boundary, EL1 saves a call continuation containing SP and x18–x30. A probe seeds and checks x18–x29 and the service stack pointer after return; a volatile sentinel on that stack is also checked. The service then enters the current EL0 frame. A QEMU virtual-timer IRQ saves the EL0 integer frame x0–x30, `SP_EL0`, `ELR_EL1` and `SPSR_EL1`; it checks task-specific register values, loop PC, user SP and SPSR/NZCV before selecting the other fixed loop. After a second IRQ, the handler deasserts the timer and completes GIC EOI, returns through `eret` to an EL1 continuation, restores the original call stack and resumes the service. IRQs remain masked while service code runs.

The IRQ path touches only preallocated context/counter state and timer/GIC registers: it does not access the graphical objects, workspace bridge, VirtIO driver, framebuffer, UART or allocator. The guest checks that its bump allocator did not advance during a slice, that both EL0 counters advanced, that exactly two IRQs were recorded, and that the timer is disabled and IRQ masked on return. These checks and the preserved integer state apply to this fixed guest configuration; they do not make arbitrary Rust reentrancy safe. The fixed two-task selection is not a general ready queue or fairness guarantee. x30 is exercised as the return path; FP/SIMD, FPCR/FPSR and architectural extensions are not saved or tested. No general EL1 interrupt, nested IRQ, SMP, task termination, blocked-task scheduling, SVC or actual EL0 protection fault is tested.

The guest inherits `system_vm`'s identity map: linked kernel image/heap/DMA are normal non-cacheable, user code and user stacks have separate permissions, and UART/GIC/VirtIO are device mappings. Caches are disabled. The QEMU HAL checks every page of a shared guest range, but has no IOMMU or DMA isolation and can share kernel writable pages including page tables and the EL1 stack. Full pointer provenance, direction, lifetime, synchronization and real-device coherence remain unproved. `mmu_status` is a post-boot assertion summary, not continuous monitoring.

## Observed VM scenario

The [modern-MMIO evidence](evidence/concurrent-vm-evidence.json) and [legacy-MMIO evidence](evidence/concurrent-vm-legacy-evidence.json) each cover eight QEMU instances. In the first guest, the runner reads `el0_status` before and after a synthetic QMP human click, an MCP assistant click and an MCP document save. It verifies both EL0 counters and the slice count increase at each checkpoint, while logical focus, pointer positions and snapshot state remain correct. A second guest similarly verifies progress around a document load. The event trace proves interleaving **between** service turns; it does not show EL0 execution inside a single MCP handler or GPU operation. Counter values are synthetic observations, not performance benchmarks.

The same run exercises full synthetic base/copy snapshots, a retained conflict, two-slot commit/stage, restart, fallback from a corrupt newest slot, rejection of two corrupt slots and refusal to overwrite an uncommitted first snapshot. The runner uses only exclusively created disposable 1 MiB images. QMP input is synthetic and framebuffer hashes establish a change, not exact geometry or physical-human input. Focus/click state is not persisted; document edits labeled human are MCP inputs, not a graphical editor.

The stage failpoint returns before the host sends SIGTERM to QEMU; it does not interrupt an individual write or simulate torn sectors, caches or power failure. A later `mode=commit` serializes the current document anew. A successful driver `flush()` return does not prove FLUSH was negotiated or reached a device. Checksums are not authentication. No crash durability, atomicity, confidentiality, general security isolation, native Apple hardware support or latency/performance guarantee is claimed.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/concurrent-vm-demo \
  --output-dir /path/to/empty-output
# Optional: repeat with --transport legacy and another empty directory
```

The host harness starts isolated one-CPU QEMU instances without network or host directory sharing, sends synthetic MCP/QMP requests and inspects only its own temporary images. It records protocol I/O, QEMU version, guest hash and framebuffer/slot hashes; it checks that the binary does not change during the run. On error, available QEMU diagnostics go to standard error. All guest code executes inside QEMU.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).
