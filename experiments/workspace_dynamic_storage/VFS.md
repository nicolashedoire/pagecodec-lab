# Bounded named files in the integrated workspace guest

The same EL1 service that owns the synthetic workspace also owns a flat catalog
of named files. `file_put`, `file_put_hex`, `file_get`, `file_list` and `file_delete` are experimental MCP tools on the
existing unauthenticated local guest serial interface. File names are 1–32
ASCII letters, digits, `.`, `_`, or `-`, excluding `.` and `..`; slashes and
directories are not supported. There are at most 8 files, at most 512 bytes
each, and at most 800 bytes in total. `file_put` accepts UTF-8 strings;
`file_put_hex` accepts even-length lowercase hexadecimal so arbitrary binary
bytes can be stored; reads return hexadecimal bytes. These limits are not a full filesystem
and requests can also fail if the shared 3,584-byte snapshot payload is full.

`init.bin` is a reserved boot-image name: `file_delete` refuses its removal,
so a confirmed deletion cannot be silently undone by boot-time seeding. A PUT
can still replace its bytes; the loader admits only the exact approved image.

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

At boot, after a blank or valid restore, the service creates the approved
`init.bin` in the in-memory catalog only when it is absent. The normal workspace
save persists it with the other files. A present file is never silently replaced.
The internal loader path borrows the file's bytes only if nonempty and at most
512 bytes, then the loader separately validates and maps the approved program;
this size check alone does not authenticate code. A full catalog can prevent
seeding. The serial endpoint is unauthenticated, and the code identity check is
limited to the approved fixture rather than a general signing system.

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

The same scenario also fills all eight catalog entries without `note`, then
boots the approved program. Its PUT fails because the catalog is full; the
process reports failure and exits, and the guest checks both code and data
aliases are removed and both frames scrubbed and released. This covers that
specific refusal path, not every disk failure or process fault.

The EL1 guard page is active only after the MMU is enabled. If the EL1 stack
has already overflowed into it, exception handling may itself fail; the guard
does not guarantee a diagnostic or recovery.
