# VM integration contracts: pointer input and workspace changes

Status: **partly integrated prototype and design proposal**, 2026-10-30. Pointer movement and synthetic document operations through the graphical guest's MCP subset have been exercised in QEMU; the broader contracts below are proposals. Concurrent changes must not be treated as integrated or validated until their code and VM evidence demonstrate otherwise. None of the contracts below establishes a security boundary by itself.

## Current evidence and gaps

| Component | What exists | What is not yet connected or proved |
| --- | --- | --- |
| [`dual_pointer_display`](../experiments/dual_pointer_display/) | A QEMU ARM guest renders two pointers, handles a VirtIO tablet and accepts an experimental subset of MCP on serial. Both movement paths dispatch through `dynamic_input`; rendering and pointer-state responses read its per-actor state. A third synthetic actor has logical MCP movement and state only, with no displayed cursor. Its MCP subset also exposes a synthetic document bridge. | The two panels are drawings, not isolated workspaces or applications. The endpoint is unauthenticated, and the pointer router and document bridge do not share an authoritative actor registry. The human button merely changes a visual indicator outside the router. |
| [`dual_input`](../experiments/dual_input/src/lib.rs) | A `no_std` router sketch handles move, focus, primary click and shared integer writes with caller-supplied grants and revisions. | The display guest does not use this router; its focus, click and shared-write paths are not connected to the display. Its actor/workspace enum is fixed to two variants; its grants are forgeable test data, not OS permissions. |
| [`dynamic_input`](../experiments/dynamic_input/src/lib.rs) | The display guest routes human and assistant movement and a third logical actor through this extensible state model; the test-only guest registers three actors. | No real actor authentication, windows, clicks or focus; registration is not exposed in MCP. The guest uses screen-global positions, while other consumers must define their own coordinate convention. |
| [`agent_pointer`](../experiments/agent_pointer/src/lib.rs) | A `no_std` adapter checks proposed assistant actions against a caller-provided snapshot and simulated grant. | It is neither an MCP server nor an authoritative dispatcher. It models button press/release and scroll, while the router models a complete primary click; blindly translating between them would change semantics. |
| [`system_core`](../experiments/system_core/src/lib.rs) | Two in-memory copies of a string-field document and a deterministic three-way merge, with field-level net-change attribution and retained conflicts. | The standalone bridge below supplies a logical edit path, but no persistence, versioned edits, actual isolation or AI conflict resolver. `edit` exposes a mutable copy, and `materialize` refuses unresolved conflicts. |
| [`workspace_bridge`](../experiments/workspace_bridge/src/lib.rs) | A QEMU guest binds two registry actor/space labels to `system_core` copies and tests edits, merge and conflict retention. The graphical guest now also instantiates the bridge behind its MCP subset. | Logical labels are not credentials; the bridge registry is separate from the graphical pointer router. No real process isolation, persistent documents, edit revisions or AI conflict resolution. |

The graphical guest uses global screen coordinates for this experiment, despite `dynamic_input` describing bounds as workspace-local: the human and assistant bounds occupy different regions of the same displayed coordinate space. This is a guest convention, not a general coordinate transform. Its third actor is logical only and has no rendered pointer or real window. Actor labels are unauthenticated, and the document bridge has a separate actor registry.

For movement, the guest uses a preallocated 64-record circular trace storing actor/space slots, coordinates and outcomes. The oldest records are overwritten and counted; unknown labels are not retained as text. The route bypasses the registry's separate unbounded event log, avoiding allocation for each movement. The [QEMU evidence](../experiments/vm_mcp_client/evidence/reproducible-vm-run.json) records 1,101 synthetic in-guest movements, then a subsequent synthetic QMP tablet event that changes the human pointer. The circular trace is not a complete or durable audit log.

The updated display guest has been exercised in QEMU with synthetic QMP tablet input, a separate MCP pointer movement and synthetic document operations via MCP. All document edits in this scenario are MCP requests, including edits labeled human; no graphical editor or human-origin editing is demonstrated. The runner checks pointer state and a changed framebuffer, separate document copies and merge results; this does not establish a multi-user desktop, application behavior, authentication or AI reasoning. See the [display experiment](../experiments/dual_pointer_display/README.md) for reproducibility and limitations.

## Proposed common event path

The following types describe intended semantics, not a promise that these Rust definitions exist:

```text
InputEnvelope {
    request_id, actor_id, workspace_id, expected_surface_revision,
    action: Move { position } | Focus { surface_id } | ...
}

VirtIO input ─┐
              ├─> actor binding → validate → serialized dispatch → accepted state → renderer
MCP request ──┘                                  │
                                                 └─> bounded trace: request + result
```

1. The compositor owns an extensible registry of actor, workspace and surface identifiers. The input adapter associates a physical or virtual input source with an actor; the MCP transport associates a session with an actor. An identifier attached by an untrusted caller must not itself confer authority.
2. Both adapters translate to the same event model. Define whether each point is workspace-local or screen-global, use checked transforms and explicit bounds, and include a surface or workspace generation to reject stale references. Do not infer permissions from the pointer's color or screen position.
3. The dispatcher checks actor/workspace binding, target ownership, bounds, the required action privilege and relevant current revision **at application time**. Its accepted state is the source for drawing and for responses; a rejected event leaves that state unchanged. The prototype can serialize this logic on a single guest event loop; actual concurrency will require equivalent atomicity.
4. A pointer move, focus change and document modification need not all share one revision counter. Define separate versions for geometry/focus and for each document so that pointer motion does not invalidate an unrelated document edit. Define precisely which changes invalidate a click or focus request.
5. Unsupported actions must receive an explicit error. In particular, no fabricated release or primary click should be inferred from a button press, and scrolling must remain unsupported until both ends implement the same semantics.

The ASCII `AI_MOVE` parser in `dual_input` is a separate experiment: the guest decodes its MCP requests and invokes `dynamic_input` directly. The MCP subset is not full protocol compliance. The dynamic path does not expose client-supplied revisions or stale-request checks. Human button input only changes a visual indicator; it is not an authorized routed window click. If framebuffer flush fails after an accepted move, the guest sends an error and halts without rolling back the router state.

## Current document bridge in QEMU

[`workspace_bridge`](../experiments/workspace_bridge/src/lib.rs) connects an `actor_core` registry containing human and assistant actor/space labels to the two in-memory `system_core` document copies. The guest applies synthetic field edits using those labels, records accepted edits and translates field-level net-change provenance to the bound actor labels. It is exercised by a standalone bare-metal QEMU program and is now also instantiated in the graphical guest. The latter exposes `document_edit`, `document_view` and `document_merge` through its MCP subset, but does not expose these operations through a functional graphical editor. The document bridge maintains its own actor registry; it is not a single shared actor registry with the pointer router.

The [standalone QEMU scenario](../experiments/workspace_bridge/evidence/workspace-bridge-evidence.json) checks that edits affect their selected copies, rejects a mismatched space and an unbound actor, verifies compatible merged values and origin labels, and retains base and both alternatives for a conflicting field. It also checks deletion provenance and a matching change attributed to both sides. Materializing a result with conflicts is refused. The [graphical guest MCP trace](../experiments/vm_mcp_client/evidence/reproducible-vm-run.json) separately demonstrates editing the two copies, reading them, merging compatible changes with origin labels, rejecting a mismatched actor/space request, and retaining the base and both alternatives for a conflict. All document edits in that trace originate from MCP, including those bearing the human label. These are checks on a small synthetic string-field document in one guest address space, not proof of authenticated authorship, application behavior or isolated workspaces. The actor and space IDs are caller-provided labels. The bridge has exactly two bound copies and does not resolve conflicts, run an AI model, enforce permissions or persist state. Attribution describes net changes, not a complete history of authorship.

## Proposed document and merge path

The separate, unconnected `dual_input` router's `WriteShared` changes a single shared `i64`, whereas `system_core` stores separate `BTreeMap<String, String>` copies. A document bridge requires a distinct contract:

```text
Edit {
    request_id, actor_id, workspace_id, document_id,
    expected_document_revision, field_key, value_or_delete
} → Applied { new_document_revision } | Rejected { reason }

Merge { document_id, base_revision, human_revision, assistant_revision }
  → fields_with_origins + retained_conflicts
```

The current graphical guest provides a bounded synthetic demonstration: its MCP input validates actor and space labels, field-key length and value length, and the bridge edits the selected copy. These labels are not credentials, and any client with access can claim either actor. A future authoritative dispatcher must verify the caller’s identity and permissions and apply an edit only to the intended workspace copy. Stale revision, unknown actor or document, oversized fields and unsupported operations should return explicit errors without changing either copy. A versioned snapshot should identify the common base and both input copies for a reproducible merge; the current document tool has no document revision or stale-edit check. The merge must not silently discard conflicts.

`system_core` can compute a deterministic first result with `Base`, `Human`, `Assistant` and `Both` labels for non-conflicting net values. Those labels do **not** prove authorship of every intermediate edit. Conflict entries retain all three values, but the existing implementation does not perform the user's desired AI-managed resolution. A future resolver needs its own structured inputs and outputs, validation, origin reporting, version checks and a retained record of the alternatives and selected resolution. Semantic correctness and provenance cannot be assumed from matching text or from a model's confidence.

Persistence, an append-only operation history, recovery after partial writes, real document formats, external side effects and application-specific semantics are separate work. A generated merge result is not yet a durable document or a transaction.

## Authority, isolation and resource limits

Today the `Grant`, `CapabilityId` and snapshots in the sketches are caller-constructible. The serial socket's private directory narrows exposure during a local demo but does not authenticate a client. The guest has no protected processes or address-space isolation. Running the proposed checks in one event loop can demonstrate behavior on synthetic data, but it cannot protect against a compromised caller in the same trust domain.

A future enforcement design needs, at minimum, an authenticated principal bound to each transport, non-forgeable scoped and revocable permissions, isolation of processes and data, and authorization and revision checks coupled atomically to the actual operation. Audit records must distinguish actor, requested action, acceptance or rejection and relevant versions, while avoiding unnecessary recording of private content. Model inference and MCP must not become implicit authority to modify the host or other actors' data.

The existing `no_std` crates use `alloc`; a bare-metal integration needs an allocator. The display guest currently uses a bounded, non-reclaiming bump allocator and stops after a bounded number of messages. Before long sessions, implement and test memory reclamation or explicit bounded lifetime, per-actor quotas, queue and payload limits, and behavior on allocation failure. Do not treat a request limit alone as production denial-of-service protection.

## Implementation and verification milestones

1. Extend the first QEMU-tested routing integration beyond movement: define extensible shared IDs, action and result types, and a strict conversion boundary for VirtIO and the MCP subset. Add explicit client-visible revision and routing outcomes before claiming stale-request protection.
2. In QEMU, extend the current synthetic human-input and MCP movement check to compare both actor states and real framebuffer changes, and exercise rejected out-of-bounds, wrong-actor, stale-revision and unsupported-action cases. Retain logs containing only synthetic inputs. A genuine physical mouse exercise is a separate observation and must be labeled as such.
3. Extend the existing graphical-guest MCP bridge: implement document revisions and reject stale edits; verify unchanged copies after every rejected operation, exercise allocation and queue limits, and log failures. Add a real UI and a genuine human input path as separate work. Do not claim AI resolution until an actual resolver is integrated and evaluated.
4. Add durable storage and recovery only after defining write ordering and verifying fault cases on disposable VM images. Follow with actual task/process isolation, trusted identities, authorization and richer application-facing protocols.
5. Extend the actor registry beyond two participants and measure resource use under representative loads; the design must not rely on a fixed actor count. Remote operation, a voice assistant and native M4 Pro hardware remain separate milestones.

All project execution tests are to run in virtual machines. VM evidence applies to the virtual devices and configuration tested, not to native Apple Silicon devices or performance.
