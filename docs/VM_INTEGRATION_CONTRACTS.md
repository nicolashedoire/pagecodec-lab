# VM integration contracts: pointer input and workspace changes

Status: **partly integrated prototype and design proposal**, 2026-10-30. A pointer-movement integration and a separate document bridge have been exercised in QEMU; the broader contracts below are proposals. Concurrent changes must not be treated as integrated or validated until their code and VM evidence demonstrate otherwise. None of the contracts below establishes a security boundary by itself.

## Current evidence and gaps

| Component | What exists | What is not yet connected or proved |
| --- | --- | --- |
| [`dual_pointer_display`](../experiments/dual_pointer_display/) | A QEMU ARM guest renders two pointers, handles a VirtIO tablet and accepts an experimental subset of MCP on serial. Both movement paths now dispatch through the `dual_input` router; rendering and pointer-state responses read its state. | The two panels are drawings, not isolated workspaces or applications. MCP currently provides movement and state only; its endpoint is unauthenticated. The human button merely changes a visual indicator outside the router. |
| [`dual_input`](../experiments/dual_input/src/lib.rs) | A `no_std` router sketch handles move, focus, primary click and shared integer writes with caller-supplied grants and revisions. | The display guest uses its movement path, but not its focus, click or shared-write paths. Its actor/workspace enum is fixed to two variants; its grants are forgeable test data, not OS permissions. |
| [`agent_pointer`](../experiments/agent_pointer/src/lib.rs) | A `no_std` adapter checks proposed assistant actions against a caller-provided snapshot and simulated grant. | It is neither an MCP server nor an authoritative dispatcher. It models button press/release and scroll, while the router models a complete primary click; blindly translating between them would change semantics. |
| [`system_core`](../experiments/system_core/src/lib.rs) | Two in-memory copies of a string-field document and a deterministic three-way merge, with field-level net-change attribution and retained conflicts. | The standalone bridge below supplies a logical edit path, but no persistence, versioned edits, actual isolation or AI conflict resolver. `edit` exposes a mutable copy, and `materialize` refuses unresolved conflicts. |
| [`workspace_bridge`](../experiments/workspace_bridge/src/lib.rs) | A separate QEMU guest binds two registry actor/space labels to `system_core` copies and tests edits, merge and conflict retention. | Logical labels are not credentials; it is not linked to the graphical guest. No real process isolation, persistent documents, edit revisions or AI conflict resolution. |

The updated display guest has been exercised in QEMU with synthetic QMP tablet input and a separate MCP pointer movement. The runner checks state and a changed framebuffer; this does not establish a multi-user desktop, application behavior, authentication or AI reasoning. See the [display experiment](../experiments/dual_pointer_display/README.md) for reproducibility and limitations.

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

The ASCII `AI_MOVE` parser in `dual_input` and the guest's JSON-RPC MCP subset are different interfaces. The guest decodes its MCP move request and invokes the router directly; it does not use that ASCII parser. The MCP subset is not full protocol compliance. The guest currently fetches the latest router revision internally immediately before dispatch; the client cannot supply an expected revision, so this does not protect a request prepared against stale client state. Human button input only changes a visual indicator; it is not an authorized routed window click. If framebuffer flush fails after an accepted move, the guest sends an error and halts without rolling back the router state.

## Current standalone document bridge

[`workspace_bridge`](../experiments/workspace_bridge/src/lib.rs) connects an `actor_core` registry containing human and assistant actor/space labels to the two in-memory `system_core` document copies. The guest applies synthetic field edits using those labels, records accepted edits and translates field-level net-change provenance to the bound actor labels. It is a separate bare-metal QEMU program, not a document service connected to the graphical guest, its MCP interface or pointer router.

The [recorded QEMU scenario](../experiments/workspace_bridge/evidence/workspace-bridge-evidence.json) checks that edits affect their selected copies, rejects a mismatched space and an unbound actor, verifies compatible merged values and origin labels, and retains base and both alternatives for a conflicting field. It also checks deletion provenance and a matching change attributed to both sides. Materializing a result with conflicts is refused. These are checks on a small synthetic string-field document in one guest address space, not proof of authenticated authorship, application behavior or isolated workspaces. The actor and space IDs are caller-provided labels. The bridge has exactly two bound copies and does not resolve conflicts, run an AI model, enforce permissions or persist state. Attribution describes net changes, not a complete history of authorship.

## Proposed document and merge path

The current router's `WriteShared` changes a single shared `i64`, whereas `system_core` stores separate `BTreeMap<String, String>` copies. A document bridge requires a distinct contract:

```text
Edit {
    request_id, actor_id, workspace_id, document_id,
    expected_document_revision, field_key, value_or_delete
} → Applied { new_document_revision } | Rejected { reason }

Merge { document_id, base_revision, human_revision, assistant_revision }
  → fields_with_origins + retained_conflicts
```

For a future integration into the graphical guest, use a bounded, synthetic document. The dispatcher must verify the target actor and workspace, then apply the edit only to that workspace's copy. Stale revision, unknown actor or document, oversized fields and unsupported operations return explicit errors without changing either copy. A versioned snapshot identifies the common base and both input copies for a reproducible merge; the merge itself must not silently discard conflicts.

`system_core` can compute a deterministic first result with `Base`, `Human`, `Assistant` and `Both` labels for non-conflicting net values. Those labels do **not** prove authorship of every intermediate edit. Conflict entries retain all three values, but the existing implementation does not perform the user's desired AI-managed resolution. A future resolver needs its own structured inputs and outputs, validation, origin reporting, version checks and a retained record of the alternatives and selected resolution. Semantic correctness and provenance cannot be assumed from matching text or from a model's confidence.

Persistence, an append-only operation history, recovery after partial writes, real document formats, external side effects and application-specific semantics are separate work. A generated merge result is not yet a durable document or a transaction.

## Authority, isolation and resource limits

Today the `Grant`, `CapabilityId` and snapshots in the sketches are caller-constructible. The serial socket's private directory narrows exposure during a local demo but does not authenticate a client. The guest has no protected processes or address-space isolation. Running the proposed checks in one event loop can demonstrate behavior on synthetic data, but it cannot protect against a compromised caller in the same trust domain.

A future enforcement design needs, at minimum, an authenticated principal bound to each transport, non-forgeable scoped and revocable permissions, isolation of processes and data, and authorization and revision checks coupled atomically to the actual operation. Audit records must distinguish actor, requested action, acceptance or rejection and relevant versions, while avoiding unnecessary recording of private content. Model inference and MCP must not become implicit authority to modify the host or other actors' data.

The existing `no_std` crates use `alloc`; a bare-metal integration needs an allocator. The display guest currently uses a bounded, non-reclaiming bump allocator and stops after a bounded number of messages. Before long sessions, implement and test memory reclamation or explicit bounded lifetime, per-actor quotas, queue and payload limits, and behavior on allocation failure. Do not treat a request limit alone as production denial-of-service protection.

## Implementation and verification milestones

1. Extend the first QEMU-tested routing integration beyond movement: define extensible shared IDs, action and result types, and a strict conversion boundary for VirtIO and the MCP subset. Add explicit client-visible revision and routing outcomes before claiming stale-request protection.
2. In QEMU, extend the current synthetic human-input and MCP movement check to compare both actor states and real framebuffer changes, and exercise rejected out-of-bounds, wrong-actor, stale-revision and unsupported-action cases. Retain logs containing only synthetic inputs. A genuine physical mouse exercise is a separate observation and must be labeled as such.
3. Connect the existing standalone bridge to the graphical guest with a bounded synthetic document and two edits; demonstrate separate copies, field-level merge, retained conflicts, rejected stale edits and unchanged state on failure. Test allocation/queue limits and log failures. Do not claim AI resolution until an actual resolver is integrated and evaluated.
4. Add durable storage and recovery only after defining write ordering and verifying fault cases on disposable VM images. Follow with actual task/process isolation, trusted identities, authorization and richer application-facing protocols.
5. Extend the actor registry beyond two participants and measure resource use under representative loads; the design must not rely on a fixed actor count. Remote operation, a voice assistant and native M4 Pro hardware remain separate milestones.

All project execution tests are to run in virtual machines. VM evidence applies to the virtual devices and configuration tested, not to native Apple Silicon devices or performance.
