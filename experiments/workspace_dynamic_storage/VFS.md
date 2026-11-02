# Bounded named files in the integrated workspace guest

The same EL1 service that owns the synthetic workspace also owns a flat catalog
of named files. `file_put`, `file_put_hex`, `file_get`, `file_list` and `file_delete` are experimental MCP tools on the
existing unauthenticated local guest serial interface. File names are 1–32
ASCII letters, digits, `.`, `_`, or `-`, excluding `.` and `..`; slashes and
directories are not supported. There are at most 10 files, at most 512 bytes
each, and at most 1,650 bytes in total. `file_put` accepts UTF-8 strings;
`file_put_hex` accepts even-length lowercase hexadecimal so arbitrary binary
bytes can be stored; reads return hexadecimal bytes. These limits are not a full filesystem
and requests can also fail if the shared 3,584-byte snapshot payload is full.

## Bounded reads from a loaded program

`SVC #0x140` accepts a UTF-8 file name address and length in `x0`/`x1`, and a
destination address and capacity in `x2`/`x3`. Each range must lie wholly in
the calling task's RW data page, its separate stack page, or its allocated heap page; the
two ranges may use different pages. Names are 1–16 bytes and destinations
1–32 bytes. The call returns a byte count or a negative status in `x0`: `-11`
means retry the same arguments, `-2` means absent/unavailable, `-34` means the
file does not fit, `-14` means an invalid user range, `-22` means invalid or
inconsistent arguments, and `-75` means request IDs are exhausted. A task can
have one read in flight. The kernel assigns the ID and checks its incarnation;
it revalidates the destination before copying and never performs a partial copy
when the destination is too small. This is an experimental read interface,
not a POSIX file descriptor API, authentication scheme, or general IPC ABI.

The integrated scenario uses two standalone AArch64 programs whose sources are
[`generic-read.S`](fixtures/generic-read.S) and
[`generic-oversize.S`](fixtures/generic-oversize.S), stores each ELF using the
VFS, restarts the same VM disk and checks its exit status. The first verifies a
successful five-byte read, missing file, small buffer, a page-crossing pointer
and neighboring guard bytes. The second verifies a present 33-byte file against
a 32-byte destination, an invalid name, all destination bytes and both guards.
The guest counts submitted/completed reads without logging file contents; the
same scenario verifies 3/3 and 2/2 completions. `fixtures/rebuild.py` verifies
the committed instruction bytes against the assembly without executing them.

## Bounded writes from a loaded program

`SVC #0x141` uses `x0`/`x1` for a 1–16 byte name and `x2`/`x3` for a
0–32 byte source in its mapped data, stack, or heap page. For a zero-length source,
the pointer may equal the end of a mapped page. GET capacity zero is
invalid. It returns the byte
count, or `-11` while pending (retry the same addresses, lengths and name bytes), `-14`
for an invalid range, `-22` for invalid arguments or a forbidden name, `-28`
for a full catalog or snapshot, `-5` for an I/O failure, and `-75` when
request IDs are exhausted. The kernel copies the bytes at submission; later
changes to the source buffer do not change that request. A task has at most one
generic read or write in flight. This is a bounded experimental ABI, not POSIX.
Changing the name while polling returns `-22` and leaves the original request
in flight until the original name is restored or the task exits.

The service stages a candidate catalog and reports success only after the
existing two-slot commit and readback. A byte-identical existing file requires
no write. A failed write after disk modification starts is ambiguous: the
guest sets `write_blocked` and suspends EL0 until reboot, so the process is not
guaranteed to receive the I/O error. Ending a process does not undo a disk
commit already performed. Reboot reconciles the two slots. Device readback is
not a guarantee of persistence across a power loss.

The integrated QEMU scenario runs [`generic-write.S`](fixtures/generic-write.S)
as a separately loaded ELF: it changes its source after submission, creates a
file and an empty file, exercises invalid names, a page-crossing source and
full-catalog refusal, then checks the original bytes and the empty file after
reboot on the same virtual disk. The existing injected post-commit error
scenario verifies the shared storage path's `write_blocked` and recovery;
it does not inject failure into the EL0 write itself.

## Two loaded processes

The integrated guest accepts `process_pair_start` with explicit file names,
for example `{"a":"left.elf","b":"right.elf"}`. Empty arguments retain the
default `pair-a.bin` and `pair-b.bin` names. Both files must already exist and
pass ELF validation before admission. Supplying the same name twice is allowed:
the two instances still receive distinct pages; the program itself must handle
being run twice. Each runs with a distinct
process incarnation and a private data and code mapping. The sample programs
write different values at the same virtual address, wait until both have been
preempted, and check their own value before exiting. The harness checks the
kernel's mapping, execution and frame-reclamation status. The scenario stores
the two ELFs through the VFS, restarts the virtual disk, rejects a malformed
ELF without partial admission, launches the corrected pair, then starts a new
pair after cleanup on the same boot. It verifies fresh incarnations, rejects
real completions from the previous pair, also rejects an explicitly constructed
token using the current request ID with the previous incarnation, and preserves
the finished result when a
new ELF is malformed. It also launches two different ELF files by explicit
names and verifies different private markers. The separate refusal while a pair is active is not
exercised by this scenario. This demonstrates
two time-sliced tasks on one virtual CPU; it does not provide a general-purpose
process manager, syscall API or multicore execution. The MCP endpoint remains
unauthenticated and is intended only for an isolated QEMU VM.

In pair mode each process can use the bounded `0x140` GET and `0x141` PUT
interfaces, with an independent outstanding request and completion. The
service checks a live actor/incarnation and permits actor A only `a-` names
and actor B only `b-` names; cross-namespace requests return `-13` without
reading or changing the catalog. This is a fixed demonstration convention,
not authentication or a general access-control model. Explicit sharing
between spaces is future work. The original single-process mode retains its
previous name rules. The integrated scenario checks both reads and writes,
denied cross-reads and cross-writes, and the committed bytes after a restart.
It also starts another pair that submits a PUT and exits without polling. A
request still queued at exit is revoked without reaching the file service;
if the service has already applied it, exit does not undo the write. The
scenario checks the outcome reported for each actor against the file contents
both before and after restart. It does not assume which side of that timing
boundary occurs. A recorded run demonstrates cancellation before application
only for actors whose `io_cancelled_before_apply` count is nonzero.

## One loaded process

`process_start` takes `{"name":"solo.elf"}` and admits one existing bounded
ELF when the task slots are idle; `process_status` reports the same latest-run
status object as `process_pair_status`, with `mode="single"` and slot 1
unused. The single process has its own identity, data and code frames. It uses
the existing bounded GET/PUT interfaces with the demonstration `a-` namespace.
The two-process barrier SVC returns `-38` in single mode. Missing files and
invalid ELF images are refused before admission.

The unified VM scenario loads [`single.S`](fixtures/single.S), verifies a
successful exit, its scoped file reads and writes, cross-namespace refusals
and frame cleanup, running a pair before and after it in the same boot, then
restarts the virtual disk to check its committed write. Concurrent admission
while the single task is active has not been forced in this scenario.
Single and pair modes remain bounded fixtures; independent admission can now use three fixed slots, not a general process API.

## Fixture arbitration

`fixtures_pause` requests that no further built-in ELF fixture be admitted;
the initial tasks and any already running fixture can finish. `fixtures_status`
reports the pause request, fixture activity, initial-task completion, admitted
image count, storage blockage and external admission readiness. A pause request
does not claim a running fixture has already stopped. `fixtures_resume` permits
remaining fixtures to resume without terminating an external task.

The QEMU scenario stores the exact bytes `1` in `fixture.pause` on its
synthetic virtual disk before a restart. On startup this explicit guest-only
configuration requests pause before fixture scheduling. The scenario waits
for the two initial tasks to finish, starts a named external ELF while the
fixture image count remains zero, deletes the configuration file, resumes,
and checks all four existing fixtures in the same boot. Other file contents do
not activate this configuration. A pause requested while a fixture is already
executing is not exercised by this scenario.

## Independently admitted processes

`process_spawn` accepts a named ELF and returns the selected free slot and its
new incarnation. `process_status` retains its historical two-slot fields and
adds `all_*` arrays for all three slots, including state, identity, I/O, exit
and frame cleanup. Three independent tasks share one virtual CPU; admission is
refused when all three slots are occupied or another execution mode is active.
The experimental `a-`, `b-` and `c-` namespace conventions follow the assigned
slot. They are not authentication or a general permissions system.

## Temporary ELF uploads

The persistent VFS retains its 512-byte per-file bound. A separate in-memory
staging area accepts one ELF of at most 16,384 bytes through `upload_begin`, sequential `upload_chunk`
requests of at most 512 bytes, then `upload_seal`. The caller supplies the
size and a canonical 16-digit FNV-1a-64 checksum. Sealing checks exact length,
checksum and the restricted ELF format; `process_spawn_staged` accepts only a
sealed image. An upload ID rejects accidental mixing of chunks from different
uploads; `upload_reset` discards the current one. Upload bytes are temporary
and disappear on reboot, and the normal VFS is not modified by staging.

FNV detects common accidental corruption, not malicious tampering. Upload IDs
are sequential and restart after a reboot, so they prevent accidental mixing
only within a boot; an old request can collide with a new upload after reboot.
They are not credentials, and the serial MCP endpoint is unauthenticated.
The guest heap uses a bounded reusable free-extent table with neighboring
extents coalesced on free. Its fixed metadata and first-fit search are designed
for this single-CPU VM, not as a general multicore allocator. The separate DMA
arena remains a one-way bump allocation and is not recycled. Raw heap
allocation can report exhaustion, but existing JSON and collection operations
may still abort the guest on OOM; the service does not promise OOM recovery.
The runner splits invalid and valid uploads across fresh boots and recreates
the volatile staging state after restart. The VM scenario uses a real compiled Rust `no_std`
program padded with unreachable ARM64 NOPs to fill the 4,096-byte code page.
It checks partial, stale, out-of-order, oversized, corrupt and invalid ELF
refusals, including an oversized and internally inconsistent segment. A
coherent 4,100-byte code segment is refused by the ELF parser after staging.
The accepted upload runs and its separate VFS write is
verified after reboot. The restricted loader is not a full native executable
environment.

The loader also accepts a restricted static AArch64 ELF with two `PT_LOAD`
segments: up to one RX code page and one RW data/BSS page. These processes use
a separate RW stack page and an unmapped guard page. The integrated scenario
uploads a real 8,552-byte Rust `no_std` ELF with initialized `.data` and zeroed
`.bss`, launches two independent copies, and verifies their data initialization,
mapping checks, distinct stack cleanup and completed file effects. Both copies
check their own data and stack contents after running concurrently. They also
exercise mixed data/stack SVC buffers, page crossings, sentinel bytes and zero
length; the PUT source snapshot is checked, though an individual run does not
prove that the mutation happened before the service applied the request. The
V1 persistent program area still accepts at most 4,216 bytes; the separate V2
area supports the larger ELF. This is a narrow ELF
subset, with no interpreter, dynamic linker or runtime relocations. Mapping
verification errors have controlled rollback, but internal MMU assertions may
still halt the guest; no general recovery from page-table corruption is claimed.

`heap_status` exposes allocation, release, live-byte, high-water and free-space
counters, and `heap_exercise` runs a bounded allocator self-check. The integrated
scenario sends 320 mixed requests after warmup, compares repeated live-byte and
largest-free snapshots, then exercises a file and a loaded process. Stability
over this finite workload does not prove the absence of leaks in other paths.

## One persistent program on a dedicated virtual disk

An explicitly provisioned disposable QEMU disk of exactly 2,048 sectors has a
separate single-program area: sectors 20–29 and 30–39. Each 5,120-byte record
contains one 512-byte header and up to 4,216 bytes of raw ELF plus zero padding.
The existing workspace snapshots remain at sectors 4–11 and 12–19, with their
format unchanged. `program_store_commit` copies a sealed staged image to the
inactive program record; `program_store_status` reports both records and
`program_store_spawn` loads the latest valid program. This is one unnamed
program slot, not a filesystem and not a transaction with the workspace.

Only blank or recognized valid records may be overwritten. An unknown or
invalid record blocks further program writes; a valid neighboring record can
still be read and run. Two blank records mean empty storage. Equal valid
generations, or no valid record when the area is nonblank, are errors. In
particular, an interrupted first write can leave invalid+blank and requires a
separate explicit recovery policy; it is never silently reformatted. A matching
magic/header does not establish ownership of arbitrary disk sectors, so this
must only be used on a virtual disk explicitly dedicated to this guest.

Commit writes an uncommitted record, flushes, writes its commit header, flushes,
then reads back and checks the entire record, checksum, padding and ELF. From
the first write until successful verification, any error blocks subsequent
snapshot and program writes until restart. Restart may recover either prior or
new committed generation after an ambiguous failure. The checksum detects
accidental corruption, not malicious changes. VirtIO flush may be a no-op if
not negotiated, and reads/writes can block; neither power-loss durability nor
responsiveness with a silent disk is established. The scenario uses temporary
disks, preserves bytes outside sectors 20–39 across program commits, checks
fallback and refusal cases, and exercises a postcommit error in test builds.

## Persistent program V2

The separate `program_store_v2_commit`, `program_store_v2_status` and
`program_store_v2_spawn` operations use two 33-sector records at 64–96 and
97–129 on the same explicitly dedicated QEMU disk. Each has a 512-byte header
and up to 16,384 bytes of ELF. V1 at 20–39 and snapshots at 4–19 remain
independent and unchanged; V1 and V2 generations are never compared and no
automatic migration or cross-version transaction is provided. A nonblank
unknown or invalid record prohibits V2 writes, even if its neighbor is valid;
a valid neighbor remains readable. Two blank records may be initialized. A
matching header is not proof of ownership: this layout is only for an
explicitly provisioned virtual disk, not an arbitrary device.

V2 uses the same stage/flush/commit/flush/full-readback pattern and shares the
write block after an ambiguous error. The scenario stores the linked Rust ELF,
also exercises an image exactly 16,384 bytes, restarts and runs the stored
program, and checks sectors outside 64–129 byte-for-byte around commits. It
tests unknown and corrupt records, blank plus interrupted record, duplicate
generations, fallback and an injected postcommit failure. Checksum is not
authentication; FLUSH may be unnegotiated, disk I/O can block, and these tests
do not establish power-loss durability or a general filesystem.

`process_stop` accepts an exact identity such as
`{"slot":0,"incarnation":"42","boot_id":"0123456789abcdef0123456789abcdef"}`.
The incarnation must be a canonical,
nonzero decimal string in the u64 range. `process_spawn` retains its numeric
field for existing clients and additionally returns `incarnation_text` and a
32-character lowercase hexadecimal `boot_id` for lossless use with
`process_stop`. The ID is a public 128-bit random value obtained asynchronously
from the guest's VirtIO RNG. If no ID is available, the service stays responsive
and rejects independent-process spawn and stop. The DMA buffer and device stay
alive if the request remains pending; there is no claimed timeout or fallback.
An old incarnation is rejected after slot reuse in the current boot, while a
different boot ID rejects the old identity even when slot and incarnation are
reused. Uniqueness depends on the host entropy source and a fresh boot; a VM
RAM snapshot or clone can reproduce the ID. This is not authentication and
the tests do not establish entropy quality. The integrated runner provisions
`virtio-rng-device` by default and also tests an absent device; a silent device
is not exercised. An external stop has outcome 4
and no process exit code;
it does not undo file effects already applied, although requests still queued
for that process are revoked. This control is not an authentication boundary.

In the integrated VM scenario, A and B wait on separate VFS files. The service
admits B while A is still running, stops A and starts a new A in its slot,
then rejects a stop using A's old identity while B continues reading. It
finally releases both via their VFS gates. Each program
checks its private marker internally before exit; the status interface does
not disclose those private bytes. The stopped instance cannot perform its
own final check. This does not establish multicore operation
or general-purpose process management. Independent tasks have no automatic
time budget: a looping task can occupy its slot until stopped by its exact
current-boot identity or until the guest restarts. The scenario does not force an I/O request to be
in flight when its neighbor is reclaimed; it also does not exercise a targeted
stop while storage writes are blocked.

`init.bin`, `fault.bin`, `badsvc.bin` and `next.bin` are reserved boot-image names: `file_delete` refuses
their removal, so a confirmed deletion cannot be silently undone by boot-time
seeding. A PUT can still replace their bytes. The loader validates a narrow ELF
format but does not authenticate code; a conforming replacement may execute.
The MCP serial interface is unauthenticated, so a client with socket access
can write a conforming ELF and have it run automatically in the guest. Run it
only in an isolated QEMU VM without networking, host directory sharing, or
physical devices. These experimental boundaries are not a production sandbox.
This is not a general program loader: the two demo tasks are also subject to
fixed register sentinels and expected SVC instruction locations. A structurally
valid ELF that does not obey these fixture rules can halt or hang the guest.

`file_put` and `file_delete` prepare a bounded candidate catalog, write it with the current
workspace to the inactive VirtIO slot, flushes, commits, flushes, and verifies
the exact slot bytes by readback. Only then does it publish the candidate in
memory and return success. `file_list` reports names and byte lengths in sorted
order. Invalid names, absent files, and catalog limits return explicit errors
without changing the in-memory catalog. The UART request line is also bounded
to 1,536 bytes including JSON overhead, enough for the maximum 512-byte file
encoded as hexadecimal under the documented schema. A failure may occur after a slot was written: no
rollback or atomicity is promised. Readback may come from a volatile device
cache and does not prove persistence across a power cut; the driver may also
complete `flush` without a negotiated FLUSH feature.

If a save fails after the first write begins, its disk outcome is ambiguous:
even a reported error can leave a valid newer committed slot. The single-CPU
service blocks further snapshot writes until a restart restores the disk. It
also blocks writes if initial slot reading or validation fails; a pair of
verified blank slots still permits initial creation. `storage_status` reports
`write_blocked` alongside `restore`; an `invalid` restore is not a successful
restore. If no virtual block device is discovered, the guest also reports
`restore=unavailable`, blocks writes and suspends the fixture; this specific
missing-device path has not been exercised in the recorded scenario. While
blocked, the fixed fixture's EL0 slices are suspended so this
QEMU guest can keep serving MCP reads. Those reads return the last in-memory
copy, which may differ from what a later restart recovers, and do not establish
general service availability or a complete recovery protocol.

An opt-in `storage-fault-test` build exposes a test-only MCP failpoint that
returns an error immediately after commit flush, before confirmation readback.
The same QEMU runner, invoked with `--exercise-storage-fault`, checks that the
unconfirmed update is absent from the in-memory view, later file writes are
blocked, and reads remain available. A fresh VM then restores the committed
slot and permits writes again. The runner also checks an image with two
nonblank invalid slots leaves all disk bytes unchanged. Both images are
temporary QEMU disks; the injection is absent from normal builds. Recorded
synthetic traces are available for [modern](evidence/storage-fault-modern.json)
and [legacy](evidence/storage-fault-legacy.json) VirtIO. Both include the
pair-to-single-to-pair execution on the current guest. This tests a specific
reported-error window, not power-loss durability or rollback.

The versioned payload has exactly `format`, `document`, and `files` keys. Its
`format` is `workspace-files-v1`; file contents are lowercase hexadecimal.
The loader also accepts the exact legacy three-key document snapshot and
starts it with an empty file catalog. Unknown/malformed formats are rejected.
Both types retain the same existing two-slot header, generation, checksum and
commit marker. A subsequent successful save writes the versioned format to the
other slot; the legacy slot is not modified by that migration save. A later
write may reuse the old slot normally.

The third fixed EL0 task uses the same bounded IPC queue to issue file PUT and
GET requests for the synthetic file `note`, containing `hello`. The service
checks the caller slot again and uses the same catalog and verified save as
MCP. PUT avoids a new write if the restored file already contains identical
bytes. GET returns at most 32 bytes through the private task page, and the task
checks the length and every returned byte before its fixed exit. The request
name is limited to 16 UTF-8 bytes. This task reuses logical actor slot 0: it
does not have independent identity or authority and does not establish a
general file API, third user, access-control model, or process isolation.
After this task exits and its code/data pages have been removed and scrubbed,
the service loads `fault.bin`, `badsvc.bin`, then `next.bin` sequentially in
fresh admissions using the same logical actor slot. The first two intentionally
trigger an invalid EL0 memory access and an unknown SVC, respectively; their
task pages are reclaimed before the following admission. The last program
writes a private marker and exits. The scenario checks all four distinct file
contents, the separate fault outcomes, and reclamation, not concurrent
execution or independent identities.

Before its invalid read, `fault.bin` submits a file PUT. Depending on timing,
the request can be serviced or cancelled when that task faults. Its completion
is revoked and scrubbed on exit. `badsvc.bin` explicitly checks that it sees no
completion from the previous incarnation before triggering its own fault. This
is a narrow reuse check for this queue and actor slot, not general identity
authentication or a complete IPC cancellation protocol. Only requests still
queued at task exit are cancelled: revoking a completion does not undo a disk
write already performed by the service. The nominal `note=hello` PUT is
idempotent in this scenario. Fault handling and cleanup are for this one-CPU
QEMU guest; they are not evidence of multicore isolation or general recovery.

The integrated scenario also replaces `badsvc.bin` with an ELF whose entry is
`movn x19,#0; b .`: x19 becomes `UINT64_MAX` and the guest confirms that it
actually observed this value at an interrupt. The guest permits 256 timer
interrupts for one loaded incarnation and terminates it on the 257th, then
reclaims its pages and runs `next.bin`. Kernel scheduling counters are separate
from EL0's register state. This is an IRQ-count limit, not a guaranteed
wall-clock deadline; it depends on timer delivery and does not cover every way
a VM can hang. The stale-completion probe is checked in the other boots,
because the looping image replaces that probe in this one.

The same integrated runner also writes a new two-instruction AArch64 ELF to
`next.bin` using the actual `file_put_hex` VFS service, restarts the same
virtual disk, then checks that its `svc #0x123` exit status in x0 is preserved
and its code and data frames are reclaimed. It repeats this with status 0 and
42. These instructions are generated by the runner, rather than selected from
kernel-linked fixture instructions. SVC #0x123 is a limited process-exit ABI,
not a general POSIX process environment or proof of arbitrary application
compatibility.

At boot, after a blank or valid restore, the service creates the four demo ELF
files in the in-memory catalog only when absent. The exact previously published
raw `init.bin` is the sole migration exception: it is replaced in memory by its
ELF equivalent, and the next normal workspace save persists the migration.
Unknown existing bytes are never silently replaced. `storage_status` reports
whether each program was present, seeded, migrated, or could not be seeded.
The internal loader borrows each file only if nonempty and at most 512 bytes,
then separately validates and maps it. A full catalog can prevent seeding.
The serial endpoint is unauthenticated, and there is no code signature check.
The accepted subset is ELF64 little-endian AArch64 ET_EXEC, with one fully
file-backed read/execute segment at the guest's fixed virtual code address;
there is no dynamic linker, relocation, writable segment, or general ELF ABI.

The integrated QEMU scenario includes the task's PUT and GET, writes and reads
`notes.txt` through MCP, creates, lists and deletes another file, verifies
refused requests, restarts and checks that the deletion persists, then corrupts
the newest slot while the VM is stopped and restores the older
valid copy including that file, and saves again. A further boot starts with a
synthetic legacy snapshot; the first migration save leaves the legacy slot
untouched, though a later successful PUT can reuse that slot.
All disks are freshly created host temporary files; no physical devices or
host folders are exposed to the guest. This is a QEMU-only experiment: there
are no directories, filesystem API, authenticated users, concurrent clients,
general process access, permissions, or guaranteed recovery from a failed
write, power loss, or malicious tampering.

The same scenario also fills all ten catalog entries without `note`, then
boots the approved program. Its PUT fails because the catalog is full; the
process reports failure and exits, and the guest checks both code and data
aliases are removed and both frames scrubbed and released. This covers that
specific refusal path, not every disk failure or process fault.

The EL1 guard page is active only after the MMU is enabled. If the EL1 stack
has already overflowed into it, exception handling may itself fail; the guard
does not guarantee a diagnostic or recovery.

### Direct IPC between independent tasks

`ipc_direct_bind` establishes one volatile, bidirectional kernel channel between
any two distinct slots among 0, 1 and 2 using their exact decimal incarnation
strings and the current 32-character boot ID. At most two distinct channels
exist simultaneously, and each still has exactly two members. Binding requires
both actors to be READY, not asleep. It returns a monotone channel ID as a decimal string;
`ipc_direct_close` requires that ID and the same boot ID. `ipc_direct_status` returns only binding state, identities,
queue lengths and counters, never message bytes or addresses. A client can observe
or bind this endpoint without authentication; boot IDs provide freshness, not
authorization. A stopped or exited peer closes the channel and clears the two
fixed FIFO queues; the explicit close operation also purges those queues.
The `scrubbed` flag describes those queue entries only, not all temporary kernel
stack copies. `slots` lists the bound members in sorted order; two-element traffic
arrays use that endpoint order, while `all_waiting` and other `all_*` wait
counters use real slot indices. `channels` reports each active channel. With two
active channels, the historical single-channel ID/slot view is null. An actor
with no live channel receives -107; an actor using a foreign or stale ID while
it belongs to another channel receives -116. This assumes one CPU, not SMP.

EL0 services `0x160`, `0x161`, and `0x162` query the peer, send, and receive.
Each destination queue holds two messages of 1–32 bytes; user buffers may be in
the actor's private data, mapped stack, or allocated heap page. The sender specifies
destination slot, incarnation and current channel ID; receives also require the
current channel ID. Stale channel IDs return -116. Empty/full queues return -11, invalid
arguments -22, an invalid buffer -14, insufficient receive capacity -34 without
consuming the message, a closed/unbound channel -107, and a stale destination
-116. Source slot and incarnation accompany successful receives. This is a
local experimental IPC primitive, not a network stack or a general capability
model.

The integrated VM scenario uses two copies of the same linked `no_std` Rust ELF.
They exchange two ordered messages each way using both stack and data buffers;
the program itself verifies payloads, sizes, boundaries, and stale destinations.
After the first actor exits, its replacement is admitted while the other stays
alive: an old incarnation is rejected before rebinding, and another exchange
checks cleanup and continued isolation. The file catalogue is unchanged.
A second fixture fills one FIFO, observes -11 on a third send, closes the
channel with two messages queued, and checks their removal. It then reopens
with a new channel ID, checks stale send/receive and stale close are rejected,
and confirms the new queue is empty. Counters coordinate the steps without
using files as a mailbox. IDs are scoped to the boot and are not authentication.

One legacy-transport full run timed out waiting for the initial MCP response
at boot 26 (15-second socket read timeout, no kernel diagnostic captured).
A subsequent run passed that boot with the same timeout. The cause is unknown;
this observation is intermittent and is not evidence of a fix.

### Experimental VirtIO Ethernet in QEMU

The integrated guest can discover two VirtIO network devices by their expected
MAC addresses and map them to any two distinct independent slots among 0–2.
For a grant, sorted slot order defines NIC ports 0 and 1; this does not add NICs
or permit multiple concurrent grants. The QEMU
scenario attaches both devices to an in-process hub with no host network
backend. It is a bounded Ethernet experiment, not IP, TCP, a general network
stack, or support for a physical Apple network adapter.

For the historical pair [0,1], legacy SVCs `0x170`/`0x171` use the currently
active grant implicitly; they do not bind the caller’s intent to a particular
generation or peer. They return -38 for other pairs. For any granted pair,
`0x174` reports its port, actual peer slot, peer incarnation and grant
generation; `0x172`/`0x173` require those expected peer and generation values
before sending or receiving one 1–64-byte private Data/Stack buffer. Stale
values return -116; a nonmember receives -13. A successful receive reports the
actual peer slot and incarnation. Buffers, lengths, live identities and boot
identity are checked. A send
queues a message in the guest; it does not itself promise delivery. The guest
encapsulates it in a private EtherType Ethernet frame and the receiver checks
the embedded boot, incarnation and grant-generation values. These identifiers
reject stale frames, but are not authentication against a hostile host or peer.
`network_status` exposes ports and counters, not packet contents.

The runner includes a boot with no NIC (the ELF must observe -19), followed by
a boot with two NICs where Rust processes observe access denial before a grant,
exchange checked ping and pong, observe closure, then exchange again after a
new grant. It checks both NICs' VirtIO counters and process cleanup. Small VFS
files synchronize the harness and are not used to carry network messages.
With the `storage-fault-test` feature, a test-only RX hold leaves a real completed
DMA buffer unread across close and reopen; releasing it checks rejection by the
new grant generation. Ordinary builds do not expose this test-only control. Frame buffers and DMA
requests have explicit lifetime requirements; this prototype does not claim
bounded response from every failed or malicious VirtIO device, including
internal driver initialization paths. It has not been tested against real
hardware and does not offer confidentiality or a complete access-control model.
There is no frame sequence number: a duplicated frame from the same boot and
same pair of incarnations can be accepted again. No within-session replay or
duplication protection is claimed.
In particular, `VirtIONetRaw::new` may loop while reading a continually changing
device configuration, so initialization is not guaranteed nonblocking for a
faulty device.
The MCP controller explicitly opens a grant for the two live slot/incarnation
identities and the current boot ID, then can close it using its decimal
generation. A peer exit also revokes the grant. These values are public freshness
checks; MCP itself is unauthenticated, so this is not protection against a
hostile client. The numeric generation in status is informational; use the
exact decimal string returned by open for close.

The experimental direct-IPC ABI has changed: `0x160` also returns the channel
ID in x2; `0x161` requires that ID in x4; `0x162` takes it in x2 and replaces
x2 with the source incarnation on success. Older IPC programs must be rebuilt
against this ABI; they cannot safely reuse the old register contract.

A blocking wait is available through SVC `0x163`: the caller supplies the
current direct-channel ID in x0 and sleeps until a message is queued. It returns
0 when data is available; the program then uses the existing receive SVC. A
closed channel wakes the waiter with -107, while a stale channel ID returns
-116. There is no deadline: without a message or closure, a task can wait
indefinitely. The controller can close the channel or stop the task. Waiting
is a kernel scheduling state, not an EL0 retry loop.

The integrated fixture checks a blocked receiver, progress by its neighbor,
message wake and receive, reblocking, wake on close, and separate cancellation
of a sleeping process. A small VFS gate controls when the sender acts; it does
not carry the IPC message. These are bounded VM scenarios on one CPU, not a
claim of general scheduling fairness.
The fixture additionally submits a GET before sleeping and checks its result
after wake. The scenario does not force the exact instant of service completion,
so it does not by itself prove that the GET completed while the task was asleep.

SVC `0x164` extends the same wait with a relative nanosecond duration: it
returns 0 for an available message, -110 for expiry, -107 for channel closure,
-116 for an old channel and -22 when the requested duration is unrepresentable.
A zero duration returns immediately and does not increment `wake_timeout`.
Timeouts are scheduling deadlines rather than real-time guarantees. Values are
converted to counter ticks with upward rounding. The guest polls deadlines even
when storage is blocked, but a woken process cannot run until dispatch resumes.
At a serialized send or close, an already due deadline wins for that waiter;
stopping a waiter instead removes that wait before channel teardown.

With `storage-fault-test`, the integrated test first expires real waits using
the QEMU ARM virtual counter. It then freezes a test-only clock, checks two
sleeping tasks before and at their deadline, advances time, verifies a message
before a later deadline, a queued message with duration zero, closure and stop.
Clock-source changes are refused during timed waits. The controlled advances
are rounded upward to counter ticks; the host timing does not establish a
real-time latency bound. A normal build does not expose test clock controls.

After a harness RPC timeout, the VM is not retried. The harness attempts to
capture the failing request, earlier RPC trace, QMP status and registers, then
asks QMP to pause the VM and copies the disposable raw disk before cleanup. The
record states whether QMP confirmed pause; otherwise the copy may be taken while
the VM is still active, and it is not a crash-consistent snapshot. QMP uses a
per-operation socket timeout; the whole diagnostic and disk copy have no global
time bound. Capture failures are recorded and the original timeout is retained.
A separate legacy run also timed out during a later request in boot 43 after
its initial graphical capture had succeeded. The failing request and VM state
were not retained. A full rerun passed boot 43 and completed with the same
15-second socket timeout; the original cause remains unknown. The new
pre-cleanup capture path was not exercised by that successful rerun.

The single-CPU scheduler selects the next ready slot by a bounded circular
search after timer interrupts, a blocking wait, or task exit. If no slot is
ready, control returns to the service loop; device polling and MCP can continue.
The guest admits up to three independent fixed contexts, with a third page-table
root (ASID 3) and a pool of thirty-three user frames. This is not SMP, arbitrary task
capacity, or a general starvation-freedom guarantee. Up to two direct IPC channels can each bind any two slots; the single two-port
network grant can also bind any two slots. These are fixed limits, not arbitrary
simultaneous channels or grants.

The integrated modern QEMU scenario runs three copies of the linked Rust ELF,
checks separate mappings, initialized private data/stacks and timer preemption,
and exercises the three per-slot namespaces, including refused cross-reads and
cross-writes. It stops and readmits A while B and C continue to make progress,
then checks each task's exit and frame cleanup. A separate integrated case has
A and B asleep on direct IPC, stops runnable C, then stops B and verifies A is
woken and all three sets of frames are reclaimed. This validates these specific
VM paths, not physical Apple drivers, application compatibility or SMP.

In feature-enabled integrated QEMU runs, an additional three-task scenario binds
1↔2 in reverse argument order, checks that the slot 0 outsider is refused,
fills and drains the FIFO, exercises message wake and timeout, then stops and
readmits slot 2. It rejects the old incarnation and channel ID before binding
0↔2, checks actual peer slot numbers, wakes and closes the new channel. Earlier
0↔1 scenarios remain in the same run. Each channel remains a bounded
two-member object.
The same feature-enabled test verifies the slot 1 outsider on the 0↔2 channel.
It does not force stopping a nonmember while a channel remains active, nor a
timed expiration with all three actors asleep. Ordinary builds do not expose
the test clock and do not run this extended three-actor scenario.

The feature-enabled network scenario also reuses the two NICs for grants 1↔2
then 0↔2. It checks real port/slot mapping, outsider refusal, legacy-SVC refusal
for those pairs, strict stale peer/generation rejection, stop and readmission.
A frame held in an actual completed RX DMA descriptor across grant replacement
is rejected after release. Earlier [0,1] runs remain in the same test. These
checks use a QEMU-only hub and do not authenticate MCP clients or imply that
legacy SVCs provide generation-bound intent.
The first modern integration run reached the final storage injection but failed
its snapshot assertion because the new network test accidentally overwrote the
runner's expected-document variable with a network-status object. The runner
now uses a distinct name. The failed run's temporary disk was removed and was
not available for inspection; this failure alone did not establish storage
corruption.

With two channels live, `0x165` resolves the peer for an explicit channel ID.
The legacy `0x160` resolves only an unambiguous membership and returns -76 for
an actor belonging to both; an actor can wait on only one channel at a time.
The feature-enabled integrated three-task scenario exchanges distinct messages on A↔B and
A↔C, closes and recreates A↔B while A↔C continues, rejects stale and foreign
IDs, and checks that stopping the common member closes both. Another case
verifies that an unrelated A↔C message and close cannot wake A waiting on A↔B.
These channels remain bounded, in-memory, unauthenticated VM mechanisms.
The extended multi-channel scenarios are recorded in the feature-enabled
modern and legacy storage-fault evidence.
The recorded feature-enabled modern and legacy traces include the explicit
readiness handshake added to the multi-channel fixture.

### Explicit document bindings for independent processes

The experimental `document_bind` associates one live independent process
incarnation with the `human` or `assistant` document copy and returns a decimal
binding ID. A process and each role have at most one active binding. The ID and
boot ID are freshness controls, not secrets or authentication: MCP remains
unauthenticated. `document_unbind` revokes a binding; stopping or exiting its
incarnation also revokes it. Slot 2 can be bound to an available role; no role
is inferred solely from a slot number.

EL0 SVCs `0x190` (GET) and `0x191` (PUT) take a 1–16-byte UTF-8 key, a private
Data/Stack value buffer of up to 32 bytes, and the exact binding ID. They share
one outstanding generic request per actor with file operations. Results include
-11 pending, -13 for no requested binding, -116 for a stale/foreign binding,
-14 for a bad user range, -22 for invalid arguments/UTF-8, -2 for a missing key,
-34 for a short GET buffer, -28 for quota and -5 for I/O failure where a reply
can be returned. The service rechecks the actor/incarnation/binding before
applying an operation; the process does not choose a role by supplying a label
in its SVC. A PUT prepares and verifies the combined document/catalog snapshot
before publishing success. A post-commit uncertainty blocks writes and can
suspend guest tasks until restart; it does not promise an immediate -5 reply.

The integrated scenario uses a disposable disk clone and two linked Rust ELFs
bound explicitly to different copies. They exercise denied access before
binding, GET/PUT/GET and role exclusivity. One is stopped and readmitted; the
old binding is rejected, the other process keeps progressing, and a new binding
updates the first copy. The scenario checks the deterministic merge's field
origins, then reboots the same disk without a later file write after the last
PUT and checks restored copies and provenance. This is a bounded two-copy
experiment; conflict resolution by an AI model, user authentication, SMP and
power-loss durability are not provided by this interface.
The recorded integrated runs cover the feature-enabled modern transport and
the ordinary-release modern and legacy transports. The feature-enabled legacy
trace predates this document-process integration.
The Bridge's in-memory event history currently grows without an eviction bound
and can exhaust the guest heap; the full event history is not persisted across
restart. Revoking queued requests or clearing an `Option` is a logical cleanup,
not evidence that every old payload copy in memory or stack was physically
wiped. The current scenario does not force revocation of an already queued
document request, quota exhaustion or a document conflict.
The current document-process scenario does not test two edits conflicting on
the same field, unbind/rebind within one incarnation, cancellation while a
document request is queued, or a document-specific post-commit failure. Merge
is deterministic field comparison with reported origins and conflicts; an AI
merge or conflict resolver is not implemented.

### Private process memory

Each independent ELF process can request up to eight extra 4 KiB private
pages. `SVC #0x182` takes an index 0–7 and returns `0x50004000 + index*4096`,
with RW and NX permissions. `SVC #0x183` frees an indexed page. An invalid
index returns `-22`, an already allocated index returns `-17`, a page that is
not allocated returns `-2`, and a lack of free frames returns `-12`; a
duplicate request does not establish that global exhaustion was exercised.
The earlier `SVC #0x180` and `SVC #0x181` allocate and free page 0, preserving
the address-based interface. Free returns `-16` if a pending generic file or
document request retains that page. Mapping a previously absent neighboring
page can also return `-16` while a zero-length one-past buffer would change
classification on polling.
Mappings and ownership are tied to the current process incarnation. Free and
process teardown remove the mapping, invalidate local translations, clear the
physical page and return the frame to the pool. These operations assume a
single guest CPU; there is no SMP shootdown or general mapping API.

The integrated [`rust-private-heap`](fixtures/rust-private-heap.rs) scenario is
designed to admit three linked Rust ELFs one at a time while earlier pages are
live. Each verifies zero bytes on allocation, a private marker at the same
virtual address, heap-backed file I/O and the busy rule; stopping and
readmitting one actor exercises frame reuse while its neighbors continue. The
ELFs free, reallocate and check all page bytes again. They also exercise an
`alloc::Vec` constructed with `from_raw_parts` after allocating through a
**fixture-only** `GlobalAlloc` that handles one whole page, and the
[guest API](guest_api.rs) for owned pages and file I/O. The scenario does not
exercise vector growth. This allocator cannot split or grow allocations and
is not an application runtime.
The integrated modern-transport run with storage fault injection and the
ordinary-release legacy run passed this scenario. They do not demonstrate
global frame exhaustion, multi-object allocation, SMP behavior or an
application runtime.

The additional [`rust-multipage-heap`](fixtures/rust-multipage-heap.rs)
scenario is designed to allocate eight pages in each of three independent ELFs,
occupying the 33-frame pool including code, data and stacks. It exercises
distinct page ownership, stopping and readmitting an actor while neighbors
keep their pages, zeroing after reallocation, and heap-backed file I/O with
the name and output on different pages. It also requests a free of both
referenced pages during that I/O and frees/reallocates an unrelated page.
Zero-length PUTs cover stack-to-heap and adjacent heap page boundaries,
including mapping a neighbor while an operation is pending. The integrated
modern-transport run with storage fault injection and the ordinary-release
legacy run passed this scenario;
filling the pool does not itself exercise a subsequent allocation returning
`-12`. Cross-calling the legacy and indexed APIs for page zero has not been
separately exercised. Programs using `0x182/183` require a kernel that
implements those calls.

### Reusable Rust object allocator

[`guest_allocator.rs`](guest_allocator.rs) supplies an opt-in `GlobalAlloc`
for independent Rust programs with one EL0 thread per address space. It keeps
up to 256 live allocation records, searches reusable gaps in acquired heap
pages, and acquires another page when necessary. A single allocation and its
alignment are each limited to 4 KiB; objects never span pages. A page owned by
an independent `HeapPage` is skipped. Allocator-acquired page owners remain in
the allocator until process exit; freeing all objects does not return those
pages early. Exhausted records, pages or too-large requests return null through
`GlobalAlloc` and may trigger an application's allocation error handler.

The atomic lock serializes allocator access under the current guest model;
it does not establish signal-handler, reentrant allocator or SMP safety. A
deallocated object can be reused immediately, even if its containing page is
still mapped. Synchronous `guest_api` calls preserve their Rust borrows while
polling, but code using asynchronous raw SVCs must retain each object until its
request completes; the page-level `-16` check does not pin individual objects.

The integrated [`rust-global-allocator`](fixtures/rust-global-allocator.rs)
fixture uses real `Box` and growing `Vec` allocations, retains a separate
`HeapPage`, verifies object extents and 256-byte alignment, releases a hole and
checks reuse, and rejects a 4,097-byte allocation. Its Vec bytes are written
through the guest file API while the objects remain live. Three ELFs run at
once; the runner stops and readmits one while the others continue, then checks
that all process-owned pages are reclaimed. The integrated ordinary-release
modern-transport QEMU series passed this scenario. The recorded legacy and
feature-enabled traces predate this allocator integration.

The first full run stopped the new fixture with exit code 9 before its gate.
Inspection of the optimized LLVM IR showed that `alloc::alloc::alloc` and its
null check had been eliminated and the fixture's failure branch was
unconditional. This did not show that the allocator returned a non-null value.
The revised fixture calls `GuestAllocator` through its `GlobalAlloc` trait
implementation and observes the returned pointer. Optimized IR confirms a
real call and null comparison; a focused QEMU run reached the 16-byte file
write, and the complete integrated modern-transport rerun passed.

### AArch64 FP/AdvSIMD context

The integrated kernel saves and restores `q0`–`q31`, `FPCR`, and `FPSR`
across guest exception boundaries, with separate EL1 and per-process EL0
states. New independent process incarnations start with a cleared FP state;
the stored process state is cleared on teardown. The guest checks the CPU's
FP/AdvSIMD feature fields and enables FP access before using these routines.
This is for the single-CPU QEMU ARM guest; it does not implement SVE or SME,
prove native Apple behavior, or establish an EL1 floating-point workload's
full isolation under arbitrary faults.

The [`fp-actors.S`](fixtures/fp-actors.S) ELF fixture is designed to inspect
all 128 bits of each vector register and both control/status registers on
entry, install distinct patterns per actor, and check them after system calls,
timer preemption, process stop/readmission, and an IPC wait/wake. It builds
with [`rebuild-fp-actors.py`](fixtures/rebuild-fp-actors.py); the source is
assembled and linked but never executed on the host. The complete integrated
modern-transport run with storage fault injection passed this scenario. It
does not include a dedicated EL1 FP-state sentinel, and recorded legacy runs
predate this change.

An earlier run was stopped before the FP scenario when a review found that
the fixture overwrote the register holding its gate name. The fixture was
corrected, rebuilt and reviewed before the successful run.

### Offset-based file access

Independent ELF processes can issue `SVC #0x142` to write a chunk and
`SVC #0x143` to read a chunk. In both cases `x0`/`x1` specify the filename,
`x2`/`x3` specify a user buffer and size, and `x4` is the byte offset. Each
chunk is at most 32 bytes; reads require a nonzero destination capacity and
writes may be empty. The [guest API](guest_api.rs) wraps these calls as
`read_at` and `write_at` and polls the same arguments through completion.

A read can return fewer bytes than requested and returns zero at or beyond
EOF, with the requested offset limited to 512. An existing file can be
overwritten in place or extended at EOF, retaining an unmodified suffix.
Creation is allowed only at offset zero; attempts to create holes return
`-22`. A write extending beyond 512 bytes, including integer overflow, returns
`-27`. Missing-file reads return `-2`, invalid user ranges `-14`, mismatched
retry arguments `-22`, and outstanding operations `-11`. Write completion
follows the same candidate snapshot and readback policy as the earlier write
interface, including its uncertain post-commit behavior and lack of proven
power-loss durability. The flat catalog, total-byte limit, lack of directories
and unauthenticated MCP interface remain unchanged.

The integrated [`rust-chunk-io`](fixtures/rust-chunk-io.rs) fixture is
designed to create a 512-byte file with sixteen writes, replace ten bytes
across an earlier chunk boundary, reject holes/overflow and invalid ranges,
then verify reads, a short final read and EOF after a VM restart on the same
disposable disk. A changed offset during polling must be rejected before the
original request is consumed. The guest Rust wrappers are exercised alongside
the raw ABI, including empty input slices and persistence of an empty file.
The integrated modern-transport QEMU run with storage fault injection passed
this scenario; the recorded legacy trace predates this integration.

### Dedicated experimental tree volume

The tree volume is separate from the legacy flat catalog and workspace
document snapshots. Its two 256-sector records occupy LBA ranges `[256,512)`
and `[512,768)` on an explicitly provisioned 2,048-sector QEMU disk. This is
not a partition map or evidence that arbitrary physical sectors are safe to
overwrite. `tree_provision` and `tree_open` require an explicit `dedicated`
assertion; blank bytes and record magic alone do not establish ownership. The
MCP control endpoint remains unauthenticated.

The tree contains at most 64 entries including root, up to 64 KiB of total
file contents and at most 16 KiB per file. Paths are canonical absolute ASCII
with a 255-byte limit and no empty, dot or dot-dot components. The service
offers `tree_mkdir`, `tree_list`, and `tree_read` (up to 512 bytes per call),
plus sequential `tree_upload_begin`, `tree_upload_append`, `tree_upload_seal`,
`tree_upload_commit` and `tree_upload_reset`. Upload chunks are at most 512
bytes; size and a 64-bit FNV checksum must match. Mutating operations require
the current generation, and a stale sealed upload cannot replace a newer tree.
These operations are MCP-facing; EL0 has separate bounded read and staged
write interfaces described below. Directory enumeration remains MCP-only.

If one uniquely selected valid copy remains beside an unknown or invalid slot,
an explicit open permits read/list in degraded, read-only mode. Mutations
remain refused and no automatic repair occurs. Two invalid copies or duplicate
valid generations cannot be selected. Commit stages and flushes a record,
commits its marker, flushes and checks readback; FNV is not authentication,
and readback does not prove atomicity under power loss or volatile caches.
The tree and legacy documents have **no shared transaction**, automatic V1
migration or synchronization. A write elsewhere after opening can also cause a
generation mismatch or block writes until restart.

The integrated runner is designed to use only disposable disk clones: it
provisions a tree, creates directories, transfers a synthetic 16 KiB file in
chunks, checks the same bytes after reboot, and compares all sectors outside
the tree region and the pre-existing documents. It also tests stale uploads,
unknown or invalid companion slots, read-only degraded opening, duplicate
generations and refusal to overwrite unrecognized regions. The complete
modern-transport run with storage fault injection passed these checks; the
recorded legacy trace predates the tree volume. It does not stage an upload
and then inject damage during the same live guest; `tree_upload_commit` in
that specific degraded transition is not covered by this scenario.

### Process reads from the tree

An independent ELF process can request `SVC #0x144` with path pointer/length
in `x0`/`x1`, destination/capacity in `x2`/`x3`, offset in `x4`, and the
nonzero tree generation in `x5`. The path is at most 255 ASCII bytes, capacity
is 1–32 bytes, and the offset is at most 16 KiB. Actor slots 0, 1 and 2 can
read only under `/a/`, `/b/` and `/c/` respectively; these fixed prefixes are
not authenticated users or general capability controls. EOF returns zero.
Errors include `-2` for an absent entry, `-13` for another actor's prefix,
`-19` for a closed tree, `-21` for a directory, `-22` for invalid arguments,
`-14` for an invalid user range, `-5` for an inconsistent disk and `-116` for
a stale generation. As with other requests, `-11` means to poll the same
arguments. The service rechecks generation and process incarnation before
replying. The user buffers remain valid only while their pages are mapped.

`SVC #0x145` reports the currently published tree generation in `x1` (`0` if
none is open), with status in `x0`. This is an advisory snapshot, not authority;
each subsequent read revalidates the tree. The [guest API](guest_api.rs)
exposes `tree_generation` and `tree_read_at`. There is no EL0
generation-change subscription yet.

The [`rust-tree-read`](fixtures/rust-tree-read.rs) integrated scenario is
designed to open a tree with two distinct 513-byte files, reboot, check that
the still-closed tree is refused, then run two independent readers in parallel.
The processes check their own complete file in chunks, EOF, namespace and
generation failures, a changed offset during polling and invalid buffer ranges.
It exercises both the raw SVC and Rust wrappers. The V1 flat catalog is used
only for test barriers, not to convey the tree generation. Actor C, degraded
EL0 reads and a tree generation change during an in-flight read are not
covered. A focused QEMU run and the complete integrated modern-transport run
with storage fault injection passed; the recorded legacy trace predates this
process-read integration. The first complete attempt exposed a missing test
barrier file and was corrected before the successful rerun.

### Process writes to the tree

An independent process can stage a tree file with `SVC #0x146`: path and
length in `x0`/`x1`, total bytes (at most 16 KiB) in `x2`, FNV-1a 64-bit
checksum in `x3`, and expected generation in `x4`. A successful BEGIN returns
a positive stage ID. `SVC #0x147` appends 1–32 bytes from a mapped user range
at the sequential offset in `x0`, with buffer/length in `x1`/`x2` and the
stage ID in `x3`. `SVC #0x148` commits and `SVC #0x149` aborts, each using
that stage ID in `x0`. A result of `-11` means retry the same arguments; the
kernel copies a submitted chunk before returning control to EL0.

Each live process incarnation has at most one stage. Stage IDs must match the
current incarnation and are never reused within a boot; they are not secrets or
authentication. The same `/a/`, `/b/`, `/c/` namespace rule applies. At
COMMIT the service rechecks the generation and only publishes after the tree
snapshot is committed and read back. A stale-generation conflict returns
`-116` and keeps the stage for explicit abort or process cleanup; a successful
commit consumes it. Other errors include `-13` wrong namespace, `-16` stage
already occupied, `-19` closed volume, `-21` directory, `-22` invalid input
or chunk order, `-27` oversized file, `-28` tree quota and `-12` allocation
failure. A process stop clears its in-memory stage; an already committed file
is not undone. An uncertain storage write can block further writes until
restart. Neither the checksum nor generation provides authentication or a
power-loss durability guarantee.

The [guest API](guest_api.rs) exposes an opt-in `TreeUpload` helper. The
[`rust-tree-write`](fixtures/rust-tree-write.rs) integrated scenario is
designed to exercise two independent writers, stale stage-ID rejection,
generation collision and retry, and stop/readmission with a partial stage. It
checks 512-byte and 16-KiB files through reboot on disposable disk clones and
uses both raw SVCs and the helper. The integrated modern-transport QEMU run
with storage fault injection passed. The recorded legacy trace predates these
process-write calls. An earlier focused run validated the two-writer collision
and cleanup before the full run.

### Process directory creation and removal

Independent ELF processes can request `SVC #0x14a` to create a directory or
`SVC #0x14b` to remove a file or empty directory. The path pointer/length are
in `x0`/`x1` (canonical ASCII path of at most 255 bytes), with the expected
nonzero tree generation in `x2`. Each call returns zero on success, or `-11`
while pending; retry the same arguments. The `/a/`, `/b/`, `/c/` actor-slot
scope applies, and a successful mutation advances the tree generation.
Failures include `-2` for a missing entry or parent, `-13` for a path outside
the caller's scope, `-17` for an existing directory, `-19` for a closed tree,
`-20` for a parent that is a file, `-22` for invalid paths/arguments, `-39`
for a nonempty directory and `-116` for a stale generation. An uncertain disk
write follows the existing blocked-until-restart policy.

The [guest API](guest_api.rs) exposes `tree_mkdir` and `tree_remove`. The
[`rust-tree-dir`](fixtures/rust-tree-dir.rs) integrated scenario is designed to
run two scoped processes, test cross-scope, nonempty-directory and stale
generation refusals, create/remove nested directories and remove a pre-existing
file, then check the resulting tree and generation again after reboot. It
exercises both SVCs and wrappers. The complete integrated modern-transport
QEMU run with storage fault injection passed. The recorded legacy trace
predates these calls. This does not provide rename, real user
authentication or power-loss durability guarantees.

### Process directory enumeration

`SVC #0x14c` enumerates one immediate child fragment at an expected tree
generation. It takes the directory path pointer/length in `x0`/`x1` (at most
255 bytes), child ordinal in `x2`, output pointer/capacity in `x3`/`x4` (2–32
bytes), generation in `x5` and byte offset within the child's name in `x6`.
Zero means end of directory when the name offset is zero. Otherwise the
positive return is one header byte followed by 1–31 name bytes, with no NUL:
header bit 0 is 0 for a directory or 1 for a file, and bit 7 means that another
fragment of this name follows. Other header bits are zero. On continuation,
keep the ordinal and generation, advance the name offset by the number of
returned name bytes, then advance the ordinal and reset the offset after the
last fragment. This permits names longer than the user buffer.

The current actor's exact root (`/a`, `/b`, or `/c`) and its descendants are
eligible; other actor scopes are refused. A stale generation returns `-116`
instead of combining directory revisions. Other failures include `-2` for a
missing directory, `-13` for another actor's scope, `-14` for invalid memory,
`-19` for a closed volume, `-20` for a file used as a directory and `-22` for
invalid arguments or a name offset beyond the selected child. EOF with a
nonzero name offset is invalid. `-11` requires polling identical arguments.
The [guest API](guest_api.rs) exposes `tree_list_fragment`; this is a bounded
experimental interface, not authenticated identity or a POSIX directory stream.

The integrated [`rust-tree-list`](fixtures/rust-tree-list.rs) scenario is
designed to exercise two independent processes, scoped roots, sorted immediate
children, exclusion of a grandchild, short buffers, a 32-byte name split into
31 and one-byte fragments, EOF, stale generations and mismatched retry
arguments. It reopens the same disposable disk after reboot and repeats the
scenario. A focused run and the complete integrated modern-transport QEMU run
with storage fault injection passed. The recorded legacy trace predates this
listing interface.

### Long process tree paths

The tree-specific process calls accept canonical ASCII paths of 1–255 bytes.
The complete user range must be contained within one mapped data, stack or
heap page; it is copied on submission and rechecked when polling. A changed
path on retry returns `-22`; a path crossing a page boundary returns `-14`,
and a 256-byte tree path returns `-22`. Pending I/O pins a heap page used by
the path against explicit page release. The flat file and document-key ABIs
retain their independent 16-byte limits.

The integrated [`rust-tree-paths`](fixtures/rust-tree-paths.rs) scenario is
designed to create, list and remove a 255-byte tree path, reject 256 bytes and
cross-page ranges, and use 33-byte paths for file creation/read/removal. It
checks another actor's prefix and verifies the old flat and document limits
using real document bindings. One actor submits its long path from a heap page,
mutates and restores it during polling, and checks that releasing the page is
refused while the request is outstanding. A second actor uses its stack. A
file in each scope is retained and read after reboot on a disposable disk.
Both a focused run and the full integrated modern-transport QEMU run with
storage fault injection passed. The 255-byte case uses one long path segment;
deeply nested paths remain untested. The recorded legacy trace predates this
path extension.
