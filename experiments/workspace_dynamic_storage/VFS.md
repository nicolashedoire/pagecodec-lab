# Bounded named files in the integrated workspace guest

The same EL1 service that owns the synthetic workspace also owns a flat catalog
of named files. `file_put`, `file_put_hex`, `file_get`, `file_list` and `file_delete` are experimental MCP tools on the
existing unauthenticated local guest serial interface. File names are 1–32
ASCII letters, digits, `.`, `_`, or `-`, excluding `.` and `..`; slashes and
directories are not supported. There are at most 10 files, at most 512 bytes
each, and at most 1,400 bytes in total. `file_put` accepts UTF-8 strings;
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
and [legacy](evidence/storage-fault-legacy.json) VirtIO. This tests a specific
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
