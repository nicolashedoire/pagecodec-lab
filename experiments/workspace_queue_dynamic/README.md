# EL0-constructed workspace requests in QEMU

This ARM64 QEMU guest extends the [fixed FIFO workspace experiment](../workspace_queue_vm/README.md). Two fixed EL0 scripts now **construct their synthetic UTF-8 strings, request IDs and memory descriptors themselves** with AArch64 stores into their own private pages. EL1 initially zeroes those pages; it no longer seeds the request strings or descriptors. The scripts then call SVC, and the kernel copies validated data to a bounded two-entry FIFO. A single EL1 graphical service drains it between slices and applies edits to two synthetic document copies. This is an experiment, not a general IPC API or authenticated OS.

## Request path

Each script writes key/value bytes at fixed offsets and six five-word descriptors at offset 512. A descriptor holds a user virtual key pointer, key length, value pointer, value length and request ID. The SVC handler identifies the actor with the checked `CURRENT` task and active TTBR, not a caller-provided role. For each fixed operation it reads the appropriate descriptor from that task's active page, checks the task-supplied ID matches the operation's expected ID, checks lengths and page-contained ranges using checked addition, copies the bytes into a fixed-size kernel request and rejects invalid UTF-8. The service independently checks encoding and bounds before editing the corresponding copy. No caller pointer is retained across a root switch.

Direct reads rely on resident, immutable QEMU mappings, a single CPU and the current experiment's interrupt discipline. They are not a fault-recovering `copy_from_user` or a security boundary for arbitrary processes. The caller IDs are constrained to this fixture's opcodes and sequence, not a reusable caller-selected ID protocol. Kernel task labels are not authentication of any real person or AI. The document MCP interfaces are read-only; attempts to invoke edit/save/load are rejected on the unauthenticated socket.

## Tests

The synthetic requests are A: `theme=human-one`, A: `theme=human-two`, and B: `layout=assistant`, with IDs 1, 2 and 3. Accepted, applied and task-confirmed completion IDs must all be `[1,2,3]`. The final document and merge provenance are checked. A sees a full queue: it reads and compares `E_FULL=3` before acknowledging it, and only then may the service drain. The full submission and duplicate-ID attempts snapshot the complete queue, completion records and associated indices/counters and require them to remain unchanged. Both tasks also generate a crossing-page range, an overflowing pointer and an invalid UTF-8 value; the kernel records both actors' rejections. Those rejection flags do not establish per-request EL0 error delivery for those cases.

Each task reads a delivered completion ID and result, compares them against its next expected ID and success, and echoes both for kernel acknowledgment. The sequence is intentionally staged, with a finite three-entry completion table; it is not a fairness or arbitrary-load test. The runner also checks rejected external MCP edit/save/load calls and exercises synthetic QMP human input and an MCP assistant click while the EL0 scripts continue.

The one-time user constructors use `x2` temporarily for arbitrary message data. An IRQ whose saved PC falls within that task's constructor permits this temporary value and checks `x3` remains zero; other register, stack, PC and flag checks remain in place. Outside construction the existing bounded transient `x2`/`x3` checks apply. The startup probe requires both tasks to progress and reach its expected state within eight timer interrupts. An unusually slow initialization can therefore cause a fixture failure; these QEMU runs do not prove all interrupt timings. FP/SIMD, arbitrary EL1 preemption, SMP, latency guarantees, IOMMU and real Apple hardware are outside scope.

The runner uses exclusively created temporary disk storage with no guest network or host directory sharing, checks the disk remains unchanged and records only synthetic interactions. The [modern](evidence/workspace-queue-dynamic-modern.json) and [legacy](evidence/workspace-queue-dynamic-legacy.json) evidence records cover the fixed scenario. They do not prove general fault recovery, security or persistence.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-queue-dynamic-demo \
  --output-dir /path/to/empty-output
# Repeat with --transport legacy and a different empty directory.
```

The harness records the guest hash before and after the run and the QEMU version. Do not rebuild during a run. Inter glyphs are distributed under the [SIL Open Font License](assets/OFL.txt).
