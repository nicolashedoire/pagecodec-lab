# Controlled cross-page EL0 read faults

This one-CPU QEMU guest extends [`address_spaces`](../address_spaces/README.md). It has two TTBR0 roots and distinct ASIDs; the same EL0 virtual alias `0x50000000` points to two different guest-physical pages. Two fixed assembly loops run in EL0 and are switched by eight virtual-timer interrupts through a preallocated ready ring. They do not voluntarily yield. This is a targeted protection experiment, not a general process implementation.

Each loop is deliberately given the **guest-physical identity address of the other loop's page**, as synthetic test data. Once per actor it executes an actual EL0 `ldr` from that address. The lower-EL synchronous handler saves integer registers and accepts only the expected actor/root, EL0 stack and mode, data-abort exception class `0x24`, level-3 permission-fault code `0x0f`, read direction, valid FAR, exact other-page address and exact controlled load PC. It records one fault per actor and advances ELR by four bytes to skip only that instruction. The user loop verifies the destination register sentinel was preserved, then continues. Unexpected faults are fatal. IRQs are masked during the synchronous handler; this is not nested-exception recovery.

As a **separate positive control within the same VM scenario**, each loop repeatedly reads and writes its own private virtual alias with a different synthetic value. Finish verifies the two expected physical page values and all other page bytes, the two fault records and ELR+4 resumes, eight IRQ selections, TTBR/ASID observations, per-task SP/SPSR and the tested integer frames. The [recorded evidence](evidence/process-isolation-evidence.json) contains normalized synthetic serial output. It demonstrates two specific denied cross-page **reads** and successful private accesses. It does **not** test a real forbidden EL0 write.

The two roots share an identical privileged kernel, UART and GIC mapping and common read-only EL0 code. Private user mappings are non-global. At every switch the guest uses barriers, writes TTBR0 and invalidates the full local EL1/EL0 TLB before resuming. This conservative single-CPU experiment does not prove safe ASID reuse, retained translations or multi-CPU shootdowns. Selected translation queries also test denial of direct physical identity and kernel addresses, but they do not cover all mappings or possible side channels.

There is no general process creation, arbitrary fault recovery, SVC dispatcher, blocking, idle path, DMA/IOMMU isolation, SMP, guard-page recovery, FP/SIMD or other extension-state preservation, real-time guarantee, or native Apple support. The identity addresses are nonsecret test values. The final scheduler state is a residual snapshot when an EL1 finish routine takes over. No performance claim is made.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/process-isolation-demo \
  --output-dir /path/to/output
```

The host starts one QEMU TCG Cortex-A57 guest with GICv2, no network, disks or host-directory sharing. The runner checks the guest hash before and after execution and records QEMU version and serial output. Do not rebuild the guest during the run. The result covers this controlled scenario, not general isolation correctness.
