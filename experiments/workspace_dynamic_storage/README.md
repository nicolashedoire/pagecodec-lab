# Integrated ARM64 guest: processes, storage and workspace prototype

This guest has grown beyond its initial fixed-message experiment. It now
includes bounded isolated ELF tasks, a reclaiming heap, IPC and sleeping tasks,
virtual storage, and an experimental isolated VirtIO Ethernet path. The
sections below retain the original experiment's history; see [VFS.md](VFS.md)
for the current interfaces, validation and limitations. It is not a complete OS
or a native Apple Silicon build.

This fixed ARM64 QEMU experiment combines [`workspace_queue_dynamic`](../workspace_queue_dynamic/) with [`workspace_storage_ipc`](../workspace_storage_ipc/) with a task-originated save. After three scripted edits have been applied and acknowledged, task A issues SVC `0x110`. The handler checks `CURRENT` and the active TTBR, accepts a save request with fixed ID 4 into the same two-entry preallocated FIFO, and never touches the document or disk. Task B issues the same opcode and receives a dedicated response with kernel-assigned fixture ID 5 and `E_DENIED=4`. B reads and compares both, then echoes them in an acknowledgment SVC; the kernel verifies the same task/TTBR and marks the denial consumed. This denial uses dedicated private-page offsets rather than the positive-completion slots. The handler verifies that rejecting B did not modify the queue, its indices or the existing completion and accepted-request records. Actor labels identify fixed synthetic tasks, not authenticated people or an AI. The fixed EL0 scripts construct the synthetic UTF-8 strings and descriptors in their own initially zeroed pages; the SVC handler validates and copies those bytes into the FIFO.

The unique EL1 service drains the FIFO between slices. For ID 4 it rechecks that the kernel-bound actor is A, serializes the current synthetic base, both copies and the bounded flat file catalog, writes the inactive slot on the test's VirtIO block device, and reads the committed slot back. It publishes a completion only after the write/flush calls return and the readback matches the exact bytes and decodes as the intended generation. Task A checks the delivered ID 4 and success code before its ACK. Submission, completed service operation and acknowledgment are distinct states. Readback is not a durability guarantee: it may be served by a volatile cache. On an error, the fixture's task expects success and stops; this is not a general recovery protocol. The unauthenticated MCP endpoint also exposes `file_get`, `file_put`, `file_list` and `file_delete` for synthetic files; document edit/save/load routes are absent. See [VFS.md](VFS.md) for limits and migration.

At startup the EL1 service reads the two synthetic slots **before** the scripted edits and restores the newest valid committed snapshot if present. A fully blank pair starts a fresh document; a nonblank pair with no valid committed slot reports invalid and leaves the fresh document unchanged. This startup restore is not requested by an EL0 SVC. `storage_status` captures the document immediately before and after this startup restore, each slot's `blank`/`valid`/`invalid` condition, and the snapshot and readback verification of the later SVC-requested save. The document bridge and disk are not accessed by the SVC or timer IRQ. The nominal SVC path performs no allocation or disk I/O; fatal diagnostic paths can write to UART.

## QEMU scenario

The integrated scenario runs multiple sequential QEMU processes with exclusively created temporary 1 MiB disk images. The first boot starts with blank slots; A and B perform their FIFO edits and save the workspace. A task loaded from `init.bin` sends FIFO PUT and GET requests for `note=hello`, checks the returned five bytes, then exits. Two further loaded programs trigger an EL0 memory fault and an unknown SVC; the guest stops each task, cleans its frames and then runs `next.bin`. Another boot exercises a looping program and its interrupt budget. Synthetic MCP requests also create, list, delete and restore files, including non-UTF-8 bytes. Recorded traces cover this flow with [modern](evidence/workspace-dynamic-storage-modern.json) and [legacy](evidence/workspace-dynamic-storage-legacy.json) VirtIO.

The [CI run for commit `0e3b6ed`](https://github.com/nicolashedoire/pagecodec-lab/actions/runs/41337845440) passed all jobs, including the integrated guest on both modern and legacy VirtIO. This result applies to that commit, not later changes.

The next boots restore the saved files, then recover from a deliberately corrupted newest slot. The first VM also receives a synthetic QMP tablet click and an MCP assistant click; neither changes the saved document. An invalid `init.bin` is refused before frames are allocated; after repair through MCP the program loads again. A subsequent full-catalog boot makes its file PUT fail and checks that the task reports failure, exits and releases its code and data frames. The last two boots exercise migration from synthetic legacy snapshots. The host terminates QEMU after each completed scenario and never attaches a physical block device or host directory to the guest.

The corruption is introduced while the VM is stopped. These checks cover that specific recovery and format migration, not interruption during a device write.

Each slot has a metadata sector and seven payload sectors. A checksum detects accidental corruption but provides no authentication. The writer stages an inactive slot then writes its commit marker; `flush()` can succeed when no FLUSH command was negotiated. These checks do **not** establish crash atomicity, durability after power loss, real-device cache behavior, or resistance to tampering. Neither mid-write interruption, disk errors nor a denied request on an otherwise idle save-only run is tested here. EL0 can write its own response/acknowledgment page, so this fixed-script acknowledgment is not authentication or a tamper-resistant audit record. The scripts and role assignments are fixed. The runner also checks both actors' crossing-page, arithmetic-overflow, duplicate-ID, invalid-UTF-8 and wrong-role rejections, plus the task-acknowledged full-queue result. These are fixed test cases, not an exhaustive adversarial test.

The one-time EL0 constructors temporarily use register x2 for message data; the IRQ checks permit that only while the saved PC is inside the relevant constructor. The startup probe expects both fixed tasks to reach the expected state within eight timer interrupts, so unusually slow initialization can fail; these runs do not cover all interrupt timings. The guest remains a one-CPU QEMU experiment: tasks are interleaved by virtual timer IRQ, EL1 yields at service-loop boundaries, and there is no arbitrary EL1 preemption or multicore parallelism. The two actual denied cross-task code reads are recovered only at their specifically expected instructions; this is not general fault recovery. The MCP socket is not authenticated, and there is no real Apple driver, IOMMU, physical DMA safety, person authentication or production persistence guarantee. The task-identity rule is a kernel-controlled fixture, not a general permissions system.

The EL1 stack has a 256 KiB reservation and one unmapped guard page once the MMU is active. A guard is not a guarantee of diagnostics or recovery: if an exception arrives with an unusable stack, handling the exception may itself fail. A large stack adjustment can also skip one guard page.

Each initial fixed task requests exit after its work is acknowledged. The kernel does not return to its EL0 frame, does not schedule it again, removes its private alias and invalidates the local TLB before zeroing and releasing its frame. Four further programs are admitted sequentially into the freed actor-0 slot. The loader checks a restricted ELF64/AArch64 format and maps a separate user read/execute page. The covered EL0 faults and an unknown SVC stop that task and scrub its code and data frames; an interrupt-count budget handles a tested infinite loop. These are bounded one-CPU scenarios, not comprehensive fault isolation or a real-time deadline. The parser does not authenticate instructions: the local unauthenticated MCP endpoint can modify automatically executed files. Other malformed behavior can still halt or hang the VM. Programs share actor 0 without independent identity or authority.

Each admission receives a kernel-controlled monotone incarnation number.
The IPC service checks actor, incarnation and request ID before an effect,
completion, delivery and acknowledgment. In this synchronous one-CPU guest,
the scenario rejects a real late completion even when the next task has reused
its request ID. This is not authenticated user identity, multicore safety or
general cancellation of an effect that has already happened.

Loaded ELF tasks can terminate with `SVC #0x123`; the kernel records the full
`x0` exit code before revoking their mappings and scrubbing their frames.
The integrated scenario writes different ELF images through the VFS, restarts
the guest and checks exit codes 0 and 42. Loaded tasks can also use the bounded
`SVC #0x140` file read with their own name and destination addresses; see
[VFS.md](VFS.md) for the ABI and its limits. Other system calls, the task slots,
and parts of the scheduler remain fixture-specific. The slice counter is now
maintained by EL1 rather than copied from the user-controlled `x19` register;
the VM exercises an interrupt with `x19` at its maximum value before running
the next program.

Loaded tasks can also request a [bounded file read](VFS.md#bounded-reads-from-a-loaded-program)
with `SVC #0x140`, supplying their own file name and destination inside their
private page. The guest checks the ranges and revalidates the incarnation and
mapping before copying a reply. This is a single-flight, nonblocking interface
for files of at most 32 bytes, not a general POSIX or multicore API.

`SVC #0x141` adds a [bounded file write](VFS.md#bounded-writes-from-a-loaded-program).
The kernel copies the submitted bytes before returning control to EL0. The VM
scenario changes the source buffer after submission and confirms the original
bytes survive a restart. If the disk outcome becomes uncertain, the existing
storage guard suspends EL0 until reboot; an immediate error return is not
guaranteed in that case.

The startup probe requires both initial tasks' counters to be nondecreasing
across interrupts and each task to progress at least once during the full
probe. An interrupt can arrive during initialization or an SVC, so equal
counter values on consecutive interrupts are not evidence of a failure. A
full modern-transport run failed the older per-interrupt condition before the
write scenario; the condition was corrected and the integrated run repeated.

The [two-loaded-process scenario](VFS.md#two-loaded-processes) admits two
different ELF files into the two existing actor slots. Each receives distinct
code and data frames and an incarnation; they write different values at the
same virtual address. A barrier opens only after both markers, both arrivals
and an interrupt for each task have been observed, then remains open while
the tasks exit. Each task can issue bounded file reads and writes through a
separate request state, with an experimental per-actor filename convention.
The kernel records exit outcomes separately from exit codes, revokes both
mappings and scrubs all four frames. This is time-slicing on one virtual CPU
with a fixture-specific barrier; it is not multicore execution, a general
process manager or an authenticated access-control system.

After both slots and the queue have been cleaned, this guest can admit another
pair during the same boot. The scenario runs successive pairs with fresh
incarnations and checks that old completion tokens are rejected. It separately
constructs a token with the current request ID but an earlier incarnation to
exercise the incarnation check itself. Refusal while an earlier pair is still
active is implemented but is not exercised deterministically by this scenario;
internal failures after allocation may still halt the guest instead of rolling
back cleanly.

The local MCP `process_pair_start` request may name both ELF files explicitly;
empty arguments retain the default fixture names. Both files are read and
validated before admission, so missing or invalid files leave the last finished
pair unchanged. Two identical file names are allowed and receive separate
frames; the behavior of the programs themselves still depends on their code.
The synchronization barrier no longer requires the fixture's specific marker
values. This remains a two-slot, one-CPU experimental interface.

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

### Rust guest applications

[`guest_api.rs`](guest_api.rs) provides a small `no_std` interface for
private heap pages and bounded file reads and writes in independent guest
processes. Its file calls retain their borrowed buffers while polling and may
wait indefinitely if the service does not progress. Names and buffers must be
in a mapped data, stack or heap page: a literal placed in the code page is not
a valid file-name buffer. `HeapPage::release` retains ownership on error;
dropping a busy page leaves it for process-exit cleanup. This is a guest ABI,
not a POSIX interface or a general allocator. The
[`rust-private-heap`](fixtures/rust-private-heap.rs) program demonstrates
this interface and constructs a `Vec` from a separately allocated raw buffer
using a fixture-only one-page allocator. It does not test vector growth.
[`guest_allocator.rs`](guest_allocator.rs) supplies a reusable bounded Rust
`GlobalAlloc` for independent processes. It retains its pages until process
exit, supports allocations fitting within one page and alignments up to 4 KiB,
and has at most 256 live allocations. Deallocation permits reuse but does not
erase the released bytes. Blocking file wrappers keep borrows alive until
completion; any future asynchronous interface must likewise retain or pin its
buffer until the operation completes.

The guest API also provides offset-based `read_at` and `write_at` calls, each
transferring up to 32 bytes. An application can use these calls to access a
larger bounded file without replacing its entire contents on every write.
Each operation is committed separately; a sequence of writes is not one atomic
transaction. The current file catalog and its persistence limits still apply.
See [VFS.md](VFS.md) for the kernel ABI, resource limits and VM evidence.

### Interactive launcher and boot identity

On macOS, after building the guest, open its QEMU window with:

```sh
python3 launch_vm.py --qemu /path/to/qemu-system-aarch64 \
  --guest /path/to/workspace-dynamic-storage-demo --display cocoa
```

The default display is `none`; use `--create-image /path/to/new.img` to create
a persistent virtual disk, or `--resume-image /path/to/existing.img` to reuse
one previously created for this guest. With neither option the disk is
temporary. The launcher prints the private MCP socket path. Its panels are a
prototype, not application windows or independent graphical desktops.

`launch_vm.py` provisions a virtual VirtIO entropy device along with its GPU,
tablet and disposable or explicitly requested persistent disk. The guest uses
entropy to create a fresh boot identity; independent process admission and
stop require that identity. The launcher does not attach a host network or
physical device. Entropy availability and random generation remain subject to
the virtual device; the identity is a freshness value, not authentication. A
[synthetic launcher trace](evidence/launcher-rng.json) records a process
admission and cleanup using a temporary VM disk.
