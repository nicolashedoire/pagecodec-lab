# Sequential EL0 preemption and graphical guest in one QEMU image

This experimental QEMU ARM64 image combines copies of [`graph_mmu`](../graph_mmu/) and the controlled timer path from [`el0_preemption`](../el0_preemption/). The same image, page tables and exception vector table are used for both stages:

1. At boot, the guest installs a narrow EL1 identity map, sets `VBAR_EL1`, then enters two fixed synthetic EL0 assembly loops on separate user stacks. Eight QEMU GICv2 virtual-timer IRQs alternate the two loops without either voluntarily yielding. The IRQ handler saves the integer register frame, `SP_EL0`, `ELR_EL1` and `SPSR_EL1`, acknowledges PPI 27, disables the timer source and issues EOI before selecting the next task.
2. After the eighth IRQ, the guest validates the recorded frames and states, disables the timer/GIC and masks interrupts. An assembly trampoline resets the EL1 stack to its known top and transfers to the graphical guest. The EL0 probe tasks are left in a residual ready/running snapshot and are never resumed during the graphical stage.
3. The graphical guest then exercises synthetic QMP tablet clicks, MCP assistant clicks, document copies and two-slot snapshot recovery. Its MCP `el0_status` and `mmu_status` expose summaries after the boot checks; they are not continuous monitors or hardware attestations.

**These stages are sequential.** The EL0 loops do not run concurrently with the graphical guest, do not operate its panels, and are not applications. There are no real windows, keyboard focus, authenticated identities, protected processes or native Apple support in this experiment. The EL0 tasks use the same address space and can access each other's mapped user stack pages; their separate stack addresses do not isolate them from one another. No syscall/SVC or actual EL0 access fault is exercised.

## Mapping and context checks

Only the linked image, its static heap/DMA regions, page tables and stacks are mapped in QEMU RAM. Kernel text is EL1 read-only/executable; kernel read-only and writable data are XN. One dedicated user-text page is EL0 readable/executable and read-only, and two dedicated user-stack pages are EL0 read/write and XN. UART, GIC distributor/CPU interface and the four pages covering VirtIO MMIO are EL1 Device-nGnRnE and XN. Instruction and data caches remain disabled. TTBR1 walks are disabled. The guest checks linked-region bounds and table alignment/non-overlap before the transition.

Address-translation (`AT`) probes check identity across the mapped image and device ranges, sample EL0 denial of reads/writes to kernel data and devices, and check that user text is readable/not writable and user-stack endpoints are readable/writable. Selected PTEs are read to check XN/Device attributes. These are checks and translation queries, **not** deliberately triggered permission or execute faults, exhaustive testing of every permission, or proof of general memory protection. The EL0 stack pages are explicitly written with zeros before the probe, but this experiment does not read back every byte.

The QEMU HAL treats guest VA as guest PA only under this identity map. It checks the full shared range and every page's translation; its permitted kernel writable region includes the kernel stack and page tables but excludes the EL0 stack pages. There is no IOMMU or DMA isolation: safe provenance, buffer lifetime, direction, ownership and synchronization remain caller/device obligations. The non-reclaiming allocator and no-op DMA release are bounded-demo choices. Caches disabled and QEMU success do not establish real-device coherence or barrier correctness.

For the fixed EL0 scenario, each IRQ record is checked for actor order, increasing per-task counter, PC inside the corresponding two-instruction loop, saved user SP, distinct per-task SPSR/NZCV and bounded EL1 IRQ stack. The final captured frames check x0–x30 except the running x19 counter, which is checked against the journal. The exception path is not a complete architecture context: FP/SIMD, FPCR/FPSR and extensions are neither saved nor tested. Unexpected exceptions halt. Nested IRQs, task completion, arbitrary task programs, multicore and continuous scheduling are not demonstrated.

## QEMU evidence

The [modern-MMIO evidence](evidence/system-vm-evidence.json) and [legacy-MMIO evidence](evidence/system-vm-legacy-evidence.json) each contain eight VM sessions. On each boot the runner checks the MMU summary and all eight validated alternating EL0 events before using the graphical MCP tools. The first VM then exercises synthetic human QMP and assistant MCP button clicks, separate logical focus, document edits and a snapshot commit. Subsequent VMs restore copies and recompute a conflict/origins, stage a newer generation before a host SIGTERM, commit a later generation, and test fallback/rejection on corrupted synthetic images. Framebuffer hashes establish changes only, not exact pointer geometry or physical-person input.

The runner creates all disk images exclusively in a private temporary directory; it accepts no pre-existing disk or physical device. The `stage` failpoint returns before commit and QEMU is terminated afterward; it does not simulate interruption during an individual write, torn sectors, device caches or power failure. A later `mode=commit` serializes the then-current snapshot anew; it does not finalize a previously staged record. A successful driver `flush()` return does not prove FLUSH was negotiated or that data reached durable storage. Checksums are not authentication. No atomicity, crash durability, confidential storage, native DMA safety or performance guarantee follows.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/system-vm-demo \
  --output-dir /path/to/empty-output
# Optional: repeat with --transport legacy and another empty directory
```

The host harness launches isolated one-CPU QEMU instances with no network or host directory sharing, sends synthetic MCP/QMP requests, and inspects only its own temporary images. It records protocol I/O, QEMU version, guest hash and framebuffer/slot hashes; it checks that the guest binary does not change during the run.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).
