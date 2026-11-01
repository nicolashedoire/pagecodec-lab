# EL0-constructed workspace messages with SVC save and restore

This fixed ARM64 QEMU experiment combines [`workspace_queue_dynamic`](../workspace_queue_dynamic/) with [`workspace_storage_ipc`](../workspace_storage_ipc/) with a task-originated save. After three scripted edits have been applied and acknowledged, task A issues SVC `0x110`. The handler checks `CURRENT` and the active TTBR, accepts a save request with fixed ID 4 into the same two-entry preallocated FIFO, and never touches the document or disk. Task B issues the same opcode and receives a dedicated response with kernel-assigned fixture ID 5 and `E_DENIED=4`. B reads and compares both, then echoes them in an acknowledgment SVC; the kernel verifies the same task/TTBR and marks the denial consumed. This denial uses dedicated private-page offsets rather than the positive-completion slots. The handler verifies that rejecting B did not modify the queue, its indices or the existing completion and accepted-request records. Actor labels identify fixed synthetic tasks, not authenticated people or an AI. The fixed EL0 scripts construct the synthetic UTF-8 strings and descriptors in their own initially zeroed pages; the SVC handler validates and copies those bytes into the FIFO.

The unique EL1 service drains the FIFO between slices. For ID 4 it rechecks that the kernel-bound actor is A, serializes the current synthetic base, both copies and the bounded flat file catalog, writes the inactive slot on the test's VirtIO block device, and reads the committed slot back. It publishes a completion only after the write/flush calls return and the readback matches the exact bytes and decodes as the intended generation. Task A checks the delivered ID 4 and success code before its ACK. Submission, completed service operation and acknowledgment are distinct states. Readback is not a durability guarantee: it may be served by a volatile cache. On an error, the fixture's task expects success and stops; this is not a general recovery protocol. The unauthenticated MCP endpoint also exposes `file_get` and `file_put` for synthetic files; document edit/save/load routes are absent. See [VFS.md](VFS.md) for limits and migration.

At startup the EL1 service reads the two synthetic slots **before** the scripted edits and restores the newest valid committed snapshot if present. A fully blank pair starts a fresh document; a nonblank pair with no valid committed slot reports invalid and leaves the fresh document unchanged. This startup restore is not requested by an EL0 SVC. `storage_status` captures the document immediately before and after this startup restore, each slot's `blank`/`valid`/`invalid` condition, and the snapshot and readback verification of the later SVC-requested save. The document bridge and disk are not accessed by the SVC or timer IRQ. The nominal SVC path performs no allocation or disk I/O; fatal diagnostic paths can write to UART.

## QEMU scenario

The scenario runs four sequential QEMU processes using exclusively created temporary 1 MiB disk images. In VM 1 the slots are blank; A and B perform their three FIFO edits, A's save writes generation 1 in slot 0, and A consumes its successful ID 4 completion. A synthetic MCP `file_put` writes `notes.txt` in generation 2 in slot 1. The runner checks B's rejected-save and task-acknowledged-denial flags and all accepted/applied/consumed IDs. It independently decodes both slots, verifies their checksums, and compares the complete copies and catalog. Recorded traces are available for [modern](evidence/workspace-dynamic-storage-modern.json) and [legacy](evidence/workspace-dynamic-storage-legacy.json) VirtIO.

VM 2 starts with that image, restores generation 2 including the named file, and saves generation 3 to slot 0. The first VM also receives a synthetic QMP tablet click and an MCP assistant click; neither changes the saved document. The runner checks slot 1 and unrelated sectors were not modified, verifies the new slot and confirms merge origins. The host terminates QEMU after each completed scenario and never attaches a physical block device or host directory to the guest. The disk files are created and removed by the host test harness.

With VM 2 stopped, the runner corrupts a payload byte in the newest slot. VM 3 reports slot 0 invalid, restores generation 2 and the named file from slot 1, and its next save replaces the invalid slot and verifies readback. VM 4 starts from a separate synthetic legacy-format disk, restores the old document with an empty catalog and writes the new format without changing the old slot. This checks a specific offline corruption and format migration, not interruption during a device write.

Each slot has a metadata sector and seven payload sectors. A checksum detects accidental corruption but provides no authentication. The writer stages an inactive slot then writes its commit marker; `flush()` can succeed when no FLUSH command was negotiated. These checks do **not** establish crash atomicity, durability after power loss, real-device cache behavior, or resistance to tampering. Neither mid-write interruption, disk errors nor a denied request on an otherwise idle save-only run is tested here. EL0 can write its own response/acknowledgment page, so this fixed-script acknowledgment is not authentication or a tamper-resistant audit record. The scripts and role assignments are fixed. The runner also checks both actors' crossing-page, arithmetic-overflow, duplicate-ID, invalid-UTF-8 and wrong-role rejections, plus the task-acknowledged full-queue result. These are fixed test cases, not an exhaustive adversarial test.

The one-time EL0 constructors temporarily use register x2 for message data; the IRQ checks permit that only while the saved PC is inside the relevant constructor. The startup probe expects both fixed tasks to reach the expected state within eight timer interrupts, so unusually slow initialization can fail; these runs do not cover all interrupt timings. The guest remains a one-CPU QEMU experiment: tasks are interleaved by virtual timer IRQ, EL1 yields at service-loop boundaries, and there is no arbitrary EL1 preemption or multicore parallelism. The two actual denied cross-task code reads are recovered only at their specifically expected instructions; this is not general fault recovery. The MCP socket is not authenticated, and there is no real Apple driver, IOMMU, physical DMA safety, person authentication or production persistence guarantee. The task-identity rule is a kernel-controlled fixture, not a general permissions system.

The EL1 stack has a 256 KiB reservation and one unmapped guard page once the MMU is active. A guard is not a guarantee of diagnostics or recovery: if an exception arrives with an unusable stack, handling the exception may itself fail. A large stack adjustment can also skip one guard page.

The two fixed EL0 tasks use separate physical code pages and separate L3 page tables. In each address space, the other task's code page is privileged and non-executable at EL0; startup checks the expected access permissions using address-translation probes. This is not a general process loader or a proof of complete isolation.

## IRQ diagnostic experiment

On a fatal IRQ check, the guest reports a phase label and the saved synthetic integer frame when that frame is available. Unexpected vectors use a separate no-frame path. Phase labels are diagnostic groupings: for example, some initial scheduler checks are in phase 1, not phase 4. These diagnostics do not identify a failed register by themselves, but include x0–x30 and the active TTBR, ELR and SPSR. Do not use this experimental guest with sensitive data.

An explicitly feature-gated test corrupts saved x5 in a separate guest build and checks the resulting phase-2 report, including `x5=42`. The [recorded injection](evidence/irq-diagnostic-evidence.json) demonstrates that diagnostic path under QEMU; it does not reproduce or explain the separate Linux CI failure in [`workspace_storage_ipc`](../workspace_storage_ipc/README.md).

```sh
CARGO_TARGET_DIR=target/irq-diagnostic cargo build --release --locked --features irq-diagnostic-test
python3 irq_diagnostic_test.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/irq-diagnostic/aarch64-unknown-none-softfloat/release/workspace-dynamic-storage-demo \
  --output-dir /path/to/empty-diagnostic-output
```

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-dynamic-storage-demo \
  --output-dir /path/to/empty-output
# Repeat with --transport legacy and another empty directory.
```

The harness records synthetic MCP I/O, guest hash and QEMU version and checks the image hash did not change. Inter glyphs are distributed under the [SIL Open Font License](assets/OFL.txt).
