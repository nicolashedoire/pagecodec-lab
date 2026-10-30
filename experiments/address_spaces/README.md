# Two TTBR0 address spaces in QEMU

This single-CPU AArch64 QEMU guest extends the fixed EL0 timer-preemption experiment in [`el0_preemption`](../el0_preemption/README.md). It constructs **two distinct TTBR0 roots**, with 8-bit ASIDs 1 and 2, and runs two synthetic EL0 loops without voluntary yields. The same user virtual address `0x50000000` maps to a different 4 KiB guest-physical page in each root. The loops use that address and the same virtual stack top but have distinct physical backing pages. The common user code is read-only/executable; each private data/stack page is read/write and execute-never.

The two roots share identical privileged-only kernel, UART and GIC mappings. Each private alias has its own L3 table; user mappings carry the non-global bit. Both physical page identities and a synthetic kernel-data address are denied to EL0 by selected `AT S1E0R/W` translation queries. User access to UART and GIC is also checked for the selected reads. This is not an actual user attempt to fault on those addresses, nor an exhaustive search of all possible mappings.

Before starting the loops, the guest zeroes both pages and confirms with EL0 translation queries that the same alias resolves to the expected distinct physical page under each root. Each EL0 loop reads and checks its own prior value, writes a distinct synthetic value, reads it back and increments a counter. Eight real virtual-timer IRQs alternate the loops through a fixed ready ring. The handler records the active TTBR and the TTBR actually selected for the next actor, saves/restores integer frames including `SP_EL0`, `ELR_EL1` and `SPSR_EL1`, and returns with `eret`. On finish, the guest checks both physical pages contain their respective values and all remaining bytes are zero, repeats the alias translations, and verifies the selected kernel/physical identity exclusions. These tests cover this specific page and sequence; they do not prove that every possible side channel or mapping is isolated.

The page tables are constructed before MMU activation and remain immutable. TCR selects TTBR0's ASID with the 8-bit ASID setting. At each root change the guest uses barriers, writes `TTBR0_EL1`, issues a **full local EL1/EL0 TLB invalidation** and completes it with barriers before resuming. The two ASIDs are distinct, but the full invalidation means this experiment does not demonstrate retention of translations across switches, safe ASID reuse or a scalable shootdown protocol. The terminal ready/running state is a residual snapshot when a separate EL1 finish function takes over.

The IRQ handler performs no allocation or serial output; virtual GIC/timer MMIO and a bounded guest-memory journal are necessary. Unexpected exceptions or invalid state stop in a nonprinting loop. This is not a complete process system, general security boundary, real-time guarantee, IOMMU/DMA isolation, multi-CPU TLB-shootdown implementation, arbitrary fault recovery or native Apple support. It does not implement SVC handling, blocking/wakeup, task creation, FP/SIMD or architectural extension-state preservation. The two user pages have no guard-page fault recovery. No performance claim is made.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/address-spaces-demo \
  --output-dir /path/to/output
```

The host launches one QEMU TCG Cortex-A57 guest with GICv2, no network, disks or host-directory sharing. The harness checks guest hash before and after execution and records QEMU version and synthetic serial output. Do not rebuild the guest during the run. See the [recorded evidence](evidence/address-spaces-evidence.json). It covers this fixed scenario only.
