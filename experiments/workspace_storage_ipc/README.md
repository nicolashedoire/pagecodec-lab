# FIFO-requested workspace save in QEMU

This fixed ARM64 QEMU experiment extends [`workspace_queue_vm`](../workspace_queue_vm/) with a task-originated save. After three scripted edits have been applied and acknowledged, task A issues SVC `0x110`. The handler checks `CURRENT` and the active TTBR, accepts a save request with fixed ID 4 into the same two-entry preallocated FIFO, and never touches the document or disk. Task B issues the same opcode and is rejected in kernel status; B is not given an individual rejection code. Actor labels identify fixed synthetic tasks, not authenticated people or an AI. The edit strings are synthetic fixtures prefilled by EL1 in the private pages.

The unique EL1 service drains the FIFO between slices. For ID 4 it rechecks that the kernel-bound actor is A, serializes the current synthetic base and both copies, writes the inactive slot on the test's VirtIO block device, then publishes a completion only after the write/flush calls return. Task A checks the delivered ID 4 and success code before its ACK. Submission, completed service operation and acknowledgment are distinct states. On an error, the fixture's task expects success and stops; this is not a general recovery protocol. The MCP endpoint is unauthenticated and only exposes document reads and storage status; document edit/save/load routes are absent.

At startup the EL1 service reads the two synthetic slots **before** the scripted edits and restores the newest valid committed snapshot if present. A fully blank pair starts a fresh document; a nonblank pair with no valid committed slot reports invalid and leaves the fresh document unchanged. This startup restore is not requested by an EL0 SVC. `storage_status` captures the document immediately before and after this startup restore, as well as the snapshot used by the later SVC-requested save. The document bridge and disk are not accessed by the SVC or timer IRQ. The nominal SVC path performs no allocation or disk I/O; fatal diagnostic paths can write to UART.

## QEMU scenario

The [modern](evidence/workspace-storage-ipc-modern.json) and [legacy](evidence/workspace-storage-ipc-legacy.json) runs each use two sequential QEMU processes with one freshly and exclusively created temporary 1 MiB disk image. In VM 1 the slots are blank; A and B perform their three FIFO edits, A's save writes generation 1 in slot 0, and A consumes its successful ID 4 completion. The runner checks B's rejected-save flag, all accepted/applied/consumed IDs and the absence of MCP mutation tools. It independently decodes the slot, verifies its checksum and compares the full base and both copies; every sector outside that slot is unchanged.

VM 2 starts with that image. The test observes a fresh initial document before startup restore and the saved copies immediately afterward, before the scripted edits run again. After the same SVC-triggered save, generation 2 is written to slot 1. The runner checks slot 0 and unrelated sectors were not modified, verifies the second slot and confirms merge origins. The host terminates QEMU after each completed scenario and never attaches a physical block device or host directory to the guest. The disk file is created and removed by the host test harness.

Each slot has a metadata sector and seven payload sectors. A checksum detects accidental corruption but provides no authentication. The writer stages an inactive slot then writes its commit marker; `flush()` can succeed when no FLUSH command was negotiated. These checks do **not** establish crash atomicity, durability after power loss, real-device cache behavior, or resistance to tampering. Neither mid-write interruption, corrupt-slot recovery, disk errors nor a denied request on an otherwise idle save-only run is tested here. The input fixture, IDs and role assignments are fixed; the full/range/spoof status from the underlying queue is not an exhaustive adversarial test.

The guest remains a one-CPU QEMU experiment: tasks are interleaved by virtual timer IRQ, EL1 yields at service-loop boundaries, and there is no arbitrary EL1 preemption or multicore parallelism. Page translation probes are not real fault recovery, the MCP socket is not authenticated, and there is no real Apple driver, IOMMU, physical DMA safety, person authentication or production persistence guarantee. The task-identity rule is a kernel-controlled fixture, not a general permissions system.

## Fatal IRQ diagnostics

Two Ubuntu CI runs produced only `F` on the guest UART before `mmu_status`. In that version several different fatal checks shared this output; the failing predicate and underlying cause are **not established**. The updated guest preserves those checks and, when a complete exception frame is available, reports a bounded synthetic guest context: family 1 covers IRQ entry and initial scheduling checks, 2 the slice register checks, 3 saved PC/status checks, and 4 slice or ready-queue scheduling checks. It includes the current actor, TTBR, EL0 SP, ELR, SPSR and saved x0–x30. Paths without a saved frame report ELR, SPSR and ESR without dereferencing a frame. A new failure may still prevent diagnostics from being emitted; these messages are not a general fault handler.

The optional `irq-diagnostic-test` build deliberately changes saved EL0 x5 to 42 before a slice. The [QEMU injection test](evidence/irq-diagnostic-evidence.json) checks that the family 2 report contains `x5=42`. This validates a diagnostic path only; it does not reproduce or explain the Ubuntu failures. Normal builds do not enable the feature. Rerunning Linux CI with this version is needed to obtain more information if the original failure recurs.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-storage-ipc-demo \
  --output-dir /path/to/empty-output
# Repeat with --transport legacy and another empty directory.

# Optional deliberate corruption test: use a separate target directory.
cargo build --release --locked --offline --features irq-diagnostic-test \
  --target-dir /path/to/diagnostic-build
python3 irq_diagnostic_test.py --qemu /path/to/qemu-system-aarch64 \
  --guest /path/to/diagnostic-build/aarch64-unknown-none-softfloat/release/workspace-storage-ipc-demo \
  --output-dir /path/to/another-empty-output
```

The harness records synthetic MCP I/O, guest hash and QEMU version and checks the image hash did not change. Inter glyphs are distributed under the [SIL Open Font License](assets/OFL.txt).
