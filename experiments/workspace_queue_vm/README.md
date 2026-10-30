# Fixed two-task FIFO workspace experiment

This ARM64 QEMU guest connects two fixed EL0 scripts to a synthetic workspace through a two-entry preallocated kernel FIFO. Three scripted requests contain bounded key and value bytes copied from the caller's private page while its own TTBR is active. The single EL1 graphical service drains the FIFO between EL0 slices, applies edits to the matching copy, and associates each completion with a kernel-derived task and request ID. It is a **fixed experiment**, not a general IPC API or an authenticated OS.

The kernel fixture initializes both private pages with synthetic strings and descriptors; the tasks trigger SVCs, but do not construct the strings. The SVC handler validates exception class, EL0 mode, task and active TTBR, reads the descriptor at a fixed index, checks key/value lengths and full page ranges with checked arithmetic, and copies the bytes into a queue entry. It retains no EL0 pointer. The service rechecks UTF-8 and lengths and alone calls the document bridge, outside the SVC/IRQ handler. Direct loads assume the test's resident, immutable QEMU mappings; they are not recoverable `copy_from_user` for arbitrary processes or faults.

The fixture binds task A to the human-labelled copy and B to the assistant-labelled copy. The bindings and request IDs 1–3 are fixed by kernel SVC opcode and script; neither proves the identity of a real person or AI. The request ID is not supplied by a general caller. The document MCP tools are read-only and the socket is unauthenticated. The prior mutation, save and load routes are absent in this guest.

## Scenario and evidence

The tasks submit three requests: A's `theme=human-one`, A's `theme=human-two`, and B's `layout=assistant`. Before the service can drain, A makes a third submission while both slots are full; the handler verifies rejection without modifying queue slots, head or length and returns `E_FULL=3`; task A reads and checks this value before the service is allowed to drain. This is an intentionally staged fixture, not a general scheduling policy. The accepted and applied IDs are `[1,2,3]`; the ring advances through its two slots and the final human value is `human-two`. B submits after A's first two completions are acknowledged. The service does not show every intermediate document version over MCP.

Each successful completion is kept in a fixed kernel record. The task invokes a polling SVC, reads the delivered ID and success/failure code from its page, copies them into acknowledgment locations and invokes an ACK SVC. The kernel matches both values to a previously delivered completion for that task and records consumption; `[1,2,3]` are consumed. The EL0 code compares each delivered ID against its expected next ID and the result against success before acknowledging it. EL0 can write its own page, so these locations are not authenticated audit records. The wrong-role opcode and crossing-page-descriptor rejections are tracked in kernel status; this script does not deliver their individual error codes to EL0.

The [modern](evidence/workspace-queue-vm-modern.json) and [legacy](evidence/workspace-queue-vm-legacy.json) runs check the empty queue, ordered accepted/applied/consumed IDs, full queue and its task-verified `E_FULL` response, and both tasks' spoof/range rejection flags, final document and merge provenance. They verify that unauthenticated MCP attempts to call edit/save/load are rejected, then inject a synthetic QMP tablet click and an MCP assistant click while the EL0 loops keep progressing. The disk is an exclusively created temporary image and remains unchanged. The host records only synthetic requests and QMP events, not a complete instruction trace.

The boundary test uses a descriptor with a key starting at the last byte of a page and length four; it does not test arithmetic overflow, invalid UTF-8, duplicate IDs, response exhaustion or device failure. Full queue handling is tested once under fixed scheduling, not under arbitrary load. The scripted owner/ID conditions, finite three-completion table and kernel-seeded pages do not provide reusable task lifecycle, unbounded queues, FIFO fairness or a production security model.

## Machine scope

The guest uses one emulated CPU. IRQs alternate the two EL0 contexts, while the service voluntarily yields at its outer-loop boundaries. Integer frame/PC/stack/root checks and the GIC/timer behavior are inherited from the earlier fixed-slice prototype; FP/SIMD, arbitrary EL1 preemption, SMP and latency guarantees are not covered. Address-translation probes do not provoke actual access faults. No real disk, physical Apple driver, host directory, private data or network is attached to the guest, and no IOMMU, hardware DMA isolation, person authentication or persistence guarantee follows from this demonstration.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-queue-vm-demo \
  --output-dir /path/to/empty-output
# Repeat with --transport legacy and a separate empty directory.
```

The host harness creates and removes its own temporary disk file, checks the guest image hash before and after and records the QEMU version. Inter glyphs are distributed under the [SIL Open Font License](assets/OFL.txt).
