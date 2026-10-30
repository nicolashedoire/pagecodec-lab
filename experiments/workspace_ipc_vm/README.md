# Fixed EL0-to-workspace SVC experiment

This QEMU ARM64 guest connects two fixed EL0 tasks to the graphical guest's synthetic document through SVC and a bounded two-slot kernel mailbox. It is a **narrow scripted prototype**, not a general IPC service, a production security boundary or a complete operating system. It follows the design diagnosis in [`workspace_ipc`](../workspace_ipc/).

Each EL0 task writes an eight-byte synthetic integer to offset 96 in its own private mapped page. The initial register values are supplied by the kernel fixture; this is not independently generated user data. Both tasks issue fixed SVC opcodes. On exception entry the handler saves the full integer frame, checks the SVC class, EL0 mode, PC within that task's code, kernel `CURRENT` and active TTBR against the expected task root. Its task-to-workspace mapping is fixed in the kernel: no claimed actor label determines authorization. The handler reads exactly eight bytes at the fixed private alias, checks the page bound and accepts only values up to 65535. It copies accepted integers into preallocated kernel state, never retains the user pointer and does not call the document bridge, allocate, log or use a device in the handler.

At the service loop boundary after an EL0 slice, the single EL1 owner reads each pending mailbox and derives a key and value from the copied integer. Task A's `0xa1a1` produces human-copy `slot-a1a1=el0-a1a1`; task B's `0xb2b2` produces assistant-copy `slot-b2b2=el0-b2b2`. The merge reports the respective human and assistant origins. Names are synthetic task roles, not verified identities of a person or AI. The drawn workspace and the MCP endpoint remain experimental, and the endpoint is unauthenticated. MCP document editing, saving and loading are not exposed in this guest; snapshot, view and merge are read-only. The QEMU disk is synthetic, created only by the test harness and remains untouched.

Each task also submits the other task's edit opcode and an unknown opcode. The handler records distinct rejection flags. Each writes a value `0x50000000` to offset 104 and invokes the reserved `0x1fe` limit-test opcode; this value is rejected as greater than 65535, and neither rejected attempt is applied to either document. The handler stores result codes in the caller's own page and the service updates the accepted result to applied. The fixed tasks do **not** read or consume these responses; `ipc_status` exposes the pages and kernel flags for the test. Because EL0 can write its own page, page response bytes are not authenticated audit records. The tests cover this script, not arbitrary adversarial task code.

The two slots are one-shot per-task bitsets with a latched payload, not a FIFO: repeated requests coalesce, there are no request IDs, reuse, queue saturation/backpressure test, reliable response delivery or generic messages. The fixed request has no user-supplied length or pointer, so cross-page and integer-overflow pointer cases are not exercised. The special `0x1fe` path is a fixture for the scalar range test, not a general ABI. EL1 uses direct loads from an immutable, already-mapped QEMU page; there is no recoverable `copy_from_user` for arbitrary mappings and faults. The handler's nonmatching identity/root path halts the guest; it is not a general syscall error path.

The inherited single-CPU scheduler alternates EL0 tasks by timer IRQ; the service voluntarily yields at loop boundaries. No IRQ touches the document or heap, and no arbitrary EL1 preemption, multicore parallelism, generic fault handling, FP/SIMD preservation, authenticated MCP caller, native Apple support, IOMMU or DMA isolation is established. AT checks probe selected EL0 translations rather than provoking faults. These fixed-task bindings do not authenticate humans and do not prevent the current actor from modifying its own page. No claim about persistent workspace restoration is made.

## QEMU evidence

The [modern](evidence/workspace-ipc-vm-modern.json) and [legacy](evidence/workspace-ipc-vm-legacy.json) VirtIO-MMIO runs each validate the two task slots, fixed payloads and private-page results, distinct workspace copies, merge origins, unrecognized MCP mutation routes, unchanged document after these rejections, EL0 progress and unchanged temporary disk. The host records synthetic MCP request/response traffic; these logs do not trace every CPU instruction or every machine I/O. Both modes run the same single guest on a single emulated CPU. No host physical device, private data, network or host directory is passed to the guest.

## Reproduce

Requires Rust target `aarch64-unknown-none-softfloat`, Python 3 and QEMU AArch64:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-ipc-vm-demo \
  --output-dir /path/to/empty-output
# Repeat with --transport legacy and a different empty output directory.
```

The host harness creates/deletes its own temporary disk file, checks the guest hash before/after and records the QEMU version. Its temporary files do not attach host block devices. The copied Inter glyphs use the [SIL Open Font License](assets/OFL.txt).
