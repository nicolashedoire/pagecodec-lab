# Two EL0 address spaces interleaved with a graphical QEMU service

This single-CPU ARM64 QEMU experiment extends [`concurrent_vm`](../concurrent_vm/) with two immutable `TTBR0_EL1` roots and ASIDs 1 and 2, following the narrower isolation experiment in [`process_isolation`](../process_isolation/). Both roots share the same privileged kernel image, page tables, heap, DMA, UART, GIC and VirtIO mappings. At EL0, each root maps the same virtual alias to a **different** synthetic physical page, while both physical identity addresses are privileged. The two fixed assembly loops share user-text code but repeatedly read/write different marker values through the common alias. Each page also provides the corresponding EL0 stack address; the loops do not exercise arbitrary stack-using programs.

The graphical EL1 service explicitly yields at outer-loop boundaries. During each slice, virtual-timer IRQs alternate the two EL0 loops, saving/restoring their integer frames, selecting the appropriate root/ASID with barriers and local `tlbi vmalle1`, then return to the same EL1 continuation under root/ASID 1. The IRQ checks the active TTBR matches the current actor. The service is **not preempted** in the middle of Rust, allocator, GPU or block operations; this is single-CPU interleaving, not multicore parallelism. A long handler or blocked device can delay the next yield indefinitely. A 20,000-poll budget bounds loop iterations, not elapsed time, and can be reached while a UART message is incomplete.

## Mapping and test scope

At boot the guest builds aligned 4 KiB page tables, keeps instruction/data caches disabled and configures shared kernel mappings plus per-root non-global user aliases and user-text entries. For each root it uses EL0 address-translation (`AT`) queries to verify that the alias resolves to its designated physical page, the neighboring alias page is unmapped, and both private physical identity addresses and sampled kernel/device addresses are unavailable to EL0. It also checks sampled kernel and VirtIO identity translations and the full mapped image, heap and DMA ranges. These are **translation queries**, not actual access faults or exhaustive permission tests.

The two EL0 pages are written with distinct synthetic markers. Each task checks its own marker in its loop; after each service slice the kernel checks both physical markers, both task counters, the restored service ASID and the timer/IRQ mask. Interrupt handling checks the actor's actual TTBR, integer register sentinels, PC range, saved user SP and SPSR/NZCV. The EL1 continuation probe checks x18–x29 and SP, return through x30, a stack sentinel and that the guest bump allocator did not advance during a slice. FP/SIMD, FPCR/FPSR, architectural extensions, nested IRQs, arbitrary programs, SMP, SVC and actual fault handling are not tested. The two-task fixed selector is not a general process scheduler or fairness guarantee.

The HAL's full-range VA=PA checks apply only to the QEMU identity-mapped guest. Its writable shareable region includes privileged stack and page-table pages; no IOMMU, ownership enforcement, authentication, confidential storage, protected application boundary or native DMA safety is provided. Safe pointer provenance, lifetimes, synchronization and real-device cache coherence remain unproved. Actor labels in graphical and document registries are unauthenticated and unrelated to the EL0 root assignment.

## QEMU evidence

The [modern-MMIO evidence](evidence/concurrent-isolation-evidence.json) and [legacy-MMIO evidence](evidence/concurrent-isolation-legacy-evidence.json) each contain eight guest sessions. Every boot checks the two-root mapping, initial EL0 timer scenario and service state. In the first VM, the runner records `el0_status` checkpoints around a synthetic QMP human click, an MCP assistant click and an MCP snapshot save; both EL0 counters advance, both private markers remain distinct, and the service reports ASID 1 at each checkpoint. A second guest checks progress around loading the snapshot. This establishes observed interleaving **between** service operations, not EL0 execution within a single MCP or VirtIO operation.

The same scenario verifies synthetic base and two document copies, conflict/origin recomputation, stage-before-commit followed by host SIGTERM, newer commit, fallback from a corrupt newest slot, rejection when both slots are corrupt, and refusal to overwrite an uncommitted first record. All disk images are exclusively created by the runner in a private temporary directory; it accepts no existing image or physical device. QMP tablet events are synthetic. The drawn buttons change logical focus/counters only; document edits are separate MCP requests, not graphical editing.

The staged failpoint is reached before QEMU is terminated; it does not interrupt an individual write or simulate torn sectors, power loss or physical caches. A later `mode=commit` serializes the current snapshot anew. A successful `flush()` return does not prove that FLUSH was negotiated or that data reached durable storage. Checksums are not authentication. No atomicity, crash durability, security certification, native Apple support or latency/performance guarantee follows. Framebuffer hashes show a change, not exact geometry or physical human input.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/concurrent-isolation-demo \
  --output-dir /path/to/empty-output
# Optional: repeat with --transport legacy and another empty directory
```

The host harness starts isolated QEMU instances without network or host directory sharing, sends synthetic MCP/QMP requests and inspects only its own temporary images. It records protocol I/O, guest hash, QEMU version, snapshot/slot and framebuffer hashes. It checks that the guest binary does not change during the run. The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).
