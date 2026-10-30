# Connecting graphical workspaces to VM storage

Status: **research proposal**, 2026-10-30. The graphical MCP guest and the persistence guest described below are separate experiments. The proposed integration, snapshot format and recovery protocol have **not** been implemented or validated, and no durability or real-device support is claimed.

## Current components and their boundaries

| Component | Current behavior | Important gap |
| --- | --- | --- |
| [`dual_pointer_display`](../experiments/dual_pointer_display/) | A QEMU guest draws pointers and serves a small MCP subset; its synthetic document bridge accepts edits, views and merges in memory. | This guest has no block device or persistent document state. Its pointer router and document actor registries are separate, and its actor labels are unauthenticated. |
| [`workspace_bridge`](../experiments/workspace_bridge/src/lib.rs) and [`system_core`](../experiments/system_core/src/lib.rs) | Two copies derive from a base; deterministic field-level merge retains conflicting values and reports net-change origins. | There is no persisted history, general document revision or AI conflict resolver. The bridge does not expose a complete versioned persistent snapshot. |
| [`workspace_persistence`](../experiments/workspace_persistence/) | A separate guest stores two fixed synthetic values in one sector and reconstructs copies in another QEMU instance. | The stored record does not contain the base, actor mapping, edit history, conflicts or origin labels. The host runner alters a synthetic control sector before starting the second VM. It is not an autonomous graphical-guest restore. |
| [`virtio_block`](../experiments/virtio_block/) and [`virtio_fix`](../experiments/virtio_fix/) | The block experiment uses a locally patched Rust VirtIO driver and disposable images, with tested modern and legacy QEMU scenarios. | Its DMA HAL and tests are specific to one QEMU guest. A successful `flush()` call does not by itself establish that the device negotiated FLUSH or that data survived power loss. |

The current persistence fixture only accepts the known `human-version` and `assistant-ready` strings. It validates a versioned 512-byte record with a non-cryptographic FNV checksum. The second guest recreates a fixed base and fixed actor-to-copy association before reapplying the saved values; it does not recover arbitrary documents or prove authenticated provenance. Its checksum and malformed-length tests do not establish transactional behavior or resistance to adversarial modification.

## Candidate integration

```text
MCP edit/view/merge → workspace bridge → bounded snapshot / status
                                               ↓
                                   virtual VirtIO block device
                                               ↓
                             disposable, runner-created VM image
```

1. Extend the graphical guest to discover an emulated VirtIO block device alongside its GPU and tablet. Use one deliberately selected driver revision and compatible transport/HAL types; the existing graphics guest and block experiment currently use separate dependency choices. Check the actual device capacity, read-only state and supported operations. A missing or unsupported disk should leave the in-memory display and editing demo available and return an explicit persistence error.
2. Define a bounded snapshot API for the bridge containing the **actual common base and both independent copies**, with document identity, format version, lengths and snapshot generation. Current private bridge state needs an intentional read/export interface; reading only merged fields would lose the alternatives for unresolved conflicts. A later format may persist actor/space labels and history, but labels are not credentials.
3. Add explicitly defined MCP operations for saving, loading and inspecting status, or specify an automatic-save contract first. Validate request arguments, sizes, field counts and device availability; return structured success or failure. Serialize a point-in-time snapshot under the single guest event loop. Before introducing true concurrent writers, specify revision checks and a lock or equivalent transaction boundary.
4. Restore by validating the complete record before changing the active in-memory state. Rebuild the two copies and recompute field-level merge results from base/human/assistant; verify conflicts and net-change origins. A corrupt or unsupported record must produce an error rather than silently resetting or overwriting data. This remains a synthetic document until a real application format and semantics are implemented.
5. Use the host harness to create images exclusively in a private temporary directory. It must not accept an existing host block device for this experiment. Attach only those synthetic images to QEMU, and keep the serial/MCP transport separate from storage data. The graphic guest's current simulated DMA assumptions, finite allocator and lack of process isolation remain constraints on any integration.

## Storage and I/O invariants

A candidate encoding should define explicit version, field count, UTF-8/byte lengths, capacity limits and deterministic ordering. All arithmetic must be checked before allocating or reading sectors; no request may read or write outside its assigned region. Failure must not partially replace the active workspace. The initial implementation should cap per-document size and field count rather than rely on available heap; errors must be observable and must not claim that unsaved data was persisted.

The user's I/O logging requirement needs an explicit contract: record operation, actor label or service, request/result, generation and loss/error information, with full payloads only for synthetic fixtures. The current pointer router has a bounded in-memory ring that overwrites older entries, while the bridge's logical events are not a persistent audit log. Neither provides a complete durable history; persistence of document snapshots does not automatically make the logs durable. Avoid collecting private document contents by default.

An example recovery design worth testing is two versioned snapshot slots. Keep the last validated slot while writing a new bounded payload and metadata to the other slot; use a separate commit indication and generation selection. On restart, reject incomplete or invalid candidates and choose a previously valid one when possible. Exact write ordering, sector atomicity, checksums and commit semantics remain to be designed and tested. A checksum detects some accidental corruption, not every corruption, and a non-cryptographic checksum does not authenticate a record.

A robust durability claim would require knowing whether the negotiated device and backing storage actually honor a flush request, and testing the required ordering under the relevant failure model. The current driver returns `Ok` when FLUSH was not negotiated; this return alone is insufficient. There is no present guarantee of atomicity, crash recovery or survival of power loss. A two-slot layout is a hypothesis to evaluate, not proof of those properties.

## Proposed VM-only verification

- Boot the **same graphical guest** with a newly created disposable image; issue synthetic MCP edits to both copies, save, stop QEMU, launch a fresh VM with the same image and restore. Compare the base, each copy, the deterministic compatible merge and a retained divergent conflict; verify unrelated pointer state remains independent.
- Exercise missing disk, read-only configuration, rejected feature negotiation, full image, invalid version, malformed lengths, invalid UTF-8, checksum corruption and unchanged sectors outside the permitted area. Confirm each error leaves the active in-memory copies and previously valid data unchanged.
- Add deterministic failpoints around writes, flush and commit, restart in a new VM and verify the last committed valid generation is selected or an explicit error is returned. Abruptly terminating QEMU models a process-level failure only; it does not reproduce physical power loss or prove the behavior of a real drive and its caches.
- Confirm structured I/O records include both successes and failures and, if a bounded log wraps, expose the lost-record count. Run the scenario with synthetic values only, inspect failure diagnostics, and retain reproducible VM evidence and the guest binary hash.

All future execution tests must remain in virtual machines. Passing them would demonstrate the selected virtual workflow, not native M4 Pro support or readiness to install on the Mac.
