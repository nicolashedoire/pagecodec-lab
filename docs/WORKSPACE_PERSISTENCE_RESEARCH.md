# Connecting graphical workspaces to VM storage

Status: **synthetic integration demonstrated; recovery remains a research proposal**, 2026-10-30. The original graphical MCP guest and fixed-field persistence guest remain separate experiments. A third guest now combines a copy of the graphical demo with bounded synthetic snapshot save and load in QEMU. No crash-durability, authentication or real-device support is claimed.

## Current components and their boundaries

| Component | Current behavior | Important gap |
| --- | --- | --- |
| [`dual_pointer_display`](../experiments/dual_pointer_display/) | A QEMU guest draws pointers and serves a small MCP subset; its synthetic document bridge accepts edits, views and merges in memory. | This guest has no block device or persistent document state. Its pointer router and document actor registries are separate, and its actor labels are unauthenticated. |
| [`graph_persistence`](../experiments/graph_persistence/) | A separate copy of the graphical demo adds VirtIO block and MCP snapshot/save/load for the actual synthetic base and both copies. | Single-slot storage, unauthenticated caller-supplied labels, no stored edit history or revision, and no transaction or crash guarantee. It is not integrated into the original graphical guest. |
| [`workspace_bridge`](../experiments/workspace_bridge/src/lib.rs) and [`system_core`](../experiments/system_core/src/lib.rs) | Two copies derive from a base; deterministic field-level merge retains conflicting values and reports net-change origins. | There is no persisted history, general document revision or AI conflict resolver. The bridge does not expose a complete versioned persistent snapshot. |
| [`workspace_persistence`](../experiments/workspace_persistence/) | A separate guest stores two fixed synthetic values in one sector and reconstructs copies in another QEMU instance. | The stored record does not contain the base, actor mapping, edit history, conflicts or origin labels. The host runner alters a synthetic control sector before starting the second VM. It is not an autonomous graphical-guest restore. |
| [`virtio_block`](../experiments/virtio_block/) and [`virtio_fix`](../experiments/virtio_fix/) | The block experiment uses a locally patched Rust VirtIO driver and disposable images, with tested modern and legacy QEMU scenarios. | Its DMA HAL and tests are specific to one QEMU guest. A successful `flush()` call does not by itself establish that the device negotiated FLUSH or that data survived power loss. |

The earlier `workspace_persistence` fixture only accepts the known `human-version` and `assistant-ready` strings. It validates a versioned 512-byte record with a non-cryptographic FNV checksum. The second guest recreates a fixed base and fixed actor-to-copy association before reapplying the saved values; it does not recover arbitrary documents or prove authenticated provenance. Its checksum and malformed-length tests do not establish transactional behavior or resistance to adversarial modification.

## QEMU graphical snapshot prototype

[`graph_persistence`](../experiments/graph_persistence/) is a **separate** guest derived from the graphical demonstration. Its MCP `document_snapshot`, `document_save` and `document_load` tools operate on one bounded synthetic document. Its versioned, single-slot 4,096-byte record stores the actual base and both copies; restoring validates a new document before replacing the active one and recomputes the merge. Fixed actor labels and bindings are recreated, while original edit history and origin labels are not stored. Reconstructed bridge events are derived from the final copies, not a replay of the original edit sequence.

The [modern QEMU evidence](../experiments/graph_persistence/evidence/graph-persistence-evidence.json) and [legacy evidence](../experiments/graph_persistence/evidence/graph-persistence-legacy-evidence.json) exercise synthetic MCP edits, deletion and conflicting changes, save, termination and a fresh VM loading the same temporary image. They check restored base/copies and recomputed conflict and origins. Separate instances reject a corrupt checksum, an incomplete but valid-checksum snapshot, and an invalid length without changing their in-memory state or image. The harness exclusively creates its 1 MiB fixtures in a private temporary directory. All document edits in this test come from MCP, including those labeled human; there is no real graphical editor or authenticated caller.

The record is **one slot**: a failed or interrupted in-place write can damage the only saved version. An `Ok` return from `flush()` does not establish that FLUSH was negotiated or that a device command was sent. The experiment does not demonstrate transactional updates, crash recovery, power-loss durability, authenticated records or confidentiality. The guest checks field and payload limits at save/load, so an edit may succeed and a later save may be rejected. It uses a finite non-reclaiming allocator and does not separately reject duplicate JSON keys. The recorded scenarios do not yet test missing/read-only disks, write errors, interruption during save or a device refusing negotiation.

## Integration beyond the synthetic prototype

```text
MCP edit/view/merge → workspace bridge → bounded snapshot / status
                                               ↓
                                   virtual VirtIO block device
                                               ↓
                             disposable, runner-created VM image
```

1. The separate `graph_persistence` guest discovers an emulated VirtIO block device alongside its GPU and tablet and uses the local patched driver. To integrate into the original graphical guest, reconcile their versions and interfaces, check actual capacity/read-only state and supported operations, and explicitly test that a missing or unsupported disk leaves the in-memory demo available with a persistence error.
2. The synthetic guest keeps the immutable base beside the bridge and reads both copies through its view API. Define a bounded snapshot API with document identity, format version, lengths and snapshot generation for a general integration. Reading only merged fields would lose alternatives for unresolved conflicts. A later format may persist actor/space labels and history, but labels are not credentials.
3. The synthetic guest exposes MCP save/load/snapshot operations. Define status and failure semantics for a general service, or specify an automatic-save contract first. Validate request arguments, sizes, field counts and device availability; return structured success or failure. Serialize a point-in-time snapshot under the single guest event loop. Before introducing true concurrent writers, specify revision checks and a lock or equivalent transaction boundary.
4. Preserve the prototype's rule of validating a complete record before changing the active in-memory state, and extend it to the full intended format. Rebuild the two copies and recompute field-level merge results from base/human/assistant; verify conflicts and net-change origins. A corrupt or unsupported record must produce an error rather than silently resetting or overwriting data. This remains a synthetic document until a real application format and semantics are implemented.
5. Continue using a host harness that creates images exclusively in a private temporary directory. It must not accept an existing host block device for this experiment. Attach only those synthetic images to QEMU, and keep the serial/MCP transport separate from storage data. The graphic guest's current simulated DMA assumptions, finite allocator and lack of process isolation remain constraints on any integration.

## Storage and I/O invariants

A candidate encoding should define explicit version, field count, UTF-8/byte lengths, capacity limits and deterministic ordering. All arithmetic must be checked before allocating or reading sectors; no request may read or write outside its assigned region. Failure must not partially replace the active workspace. The initial implementation should cap per-document size and field count rather than rely on available heap; errors must be observable and must not claim that unsaved data was persisted.

The user's I/O logging requirement needs an explicit contract: record operation, actor label or service, request/result, generation and loss/error information, with full payloads only for synthetic fixtures. The current pointer router has a bounded in-memory ring that overwrites older entries, while the bridge's logical events are not a persistent audit log. Neither provides a complete durable history; persistence of document snapshots does not automatically make the logs durable. Avoid collecting private document contents by default.

An example recovery design worth testing is two versioned snapshot slots. Keep the last validated slot while writing a new bounded payload and metadata to the other slot; use a separate commit indication and generation selection. On restart, reject incomplete or invalid candidates and choose a previously valid one when possible. Exact write ordering, sector atomicity, checksums and commit semantics remain to be designed and tested. A checksum detects some accidental corruption, not every corruption, and a non-cryptographic checksum does not authenticate a record.

A robust durability claim would require knowing whether the negotiated device and backing storage actually honor a flush request, and testing the required ordering under the relevant failure model. The current driver returns `Ok` when FLUSH was not negotiated; this return alone is insufficient. There is no present guarantee of atomicity, crash recovery or survival of power loss. A two-slot layout is a hypothesis to evaluate, not proof of those properties.

## Proposed VM-only verification

- Extend the existing `graph_persistence` QEMU test, which already saves synthetic MCP edits, restores the base and both copies in a fresh instance, and checks conflicts/origins and pointer state. Integrate an equivalent flow into the original graphical guest only after its storage interface is designed and verified.
- Exercise missing disk, read-only configuration, rejected feature negotiation, full image, invalid version, malformed lengths, invalid UTF-8, checksum corruption and unchanged sectors outside the permitted area. Confirm each error leaves the active in-memory copies and previously valid data unchanged.
- Add deterministic failpoints around writes, flush and commit, restart in a new VM and verify the last committed valid generation is selected or an explicit error is returned. Abruptly terminating QEMU models a process-level failure only; it does not reproduce physical power loss or prove the behavior of a real drive and its caches.
- Confirm structured I/O records include both successes and failures and, if a bounded log wraps, expose the lost-record count. Run the scenario with synthetic values only, inspect failure diagnostics, and retain reproducible VM evidence and the guest binary hash.

All future execution tests must remain in virtual machines. Passing them would demonstrate the selected virtual workflow, not native M4 Pro support or readiness to install on the Mac.
