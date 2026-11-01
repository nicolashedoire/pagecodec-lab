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
destination address and capacity in `x2`/`x3`. Both ranges must be inside the
calling task's mapped private page; names are 1–16 bytes and destinations
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
0–32 byte source within the calling task's private page. It returns the byte
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
This remains a fixed two-slot experimental guest, not a general process API.

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
new incarnation. `process_status` reports each slot's state, identity, I/O,
exit and frame cleanup. Two independent tasks share a single virtual CPU;
admission is refused if both slots are occupied or another execution mode is
active. The experimental `a-` and `b-` namespace conventions follow the
assigned slot. They are not a general permissions system.

## Temporary ELF uploads

The persistent VFS retains its 512-byte per-file bound. A separate in-memory
staging area accepts one ELF of at most 4,216 bytes (a 120-byte header and up
to 4,096 bytes of code) through `upload_begin`, sequential `upload_chunk`
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
coherent 4,100-byte code image is refused by the upload-size limit before the
ELF parser sees it. The accepted upload runs and its separate VFS write is
verified after reboot. The restricted loader is not a full native executable
environment.

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
