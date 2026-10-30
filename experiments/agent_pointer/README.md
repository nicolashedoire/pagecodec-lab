# AI pointer: typed adapter contract

This is a **no_std + alloc, user-space/guest-compatible design sketch**, not a
working OS input driver, actual MCP server, window manager, security boundary,
or evidence that the current Mac supports two cursors or independent focus.
It does not read machine state or emit input events. Its only output is a
structured trace with either a proposed event or a rejection.

## Data flow

A future MCP adapter could decode an authorized request into `AgentCommand`.
A future router would provide a `PointerSnapshot` from its own trusted state,
and a protected authority would evaluate a grant. The example function
`adapt(command, snapshot, grant)` compares simulated values and returns one
`TraceRecord`; the accepted variant contains a `PointerEvent` tagged
`Workspace::Assistant`.

- `AgentCommand`: request identifier, illustrative capability ID, requested
  workspace and window, expected state revision and typed action.
- `PointerSnapshot`: current workspace, target window, revision, half-open
  window bounds and assistant pointer position.
- `SimulatedGrant`: illustrative capability ID, assistant workspace, authorized
  windows and permitted action kinds (move, button, scroll).
- `TraceRecord`: complete caller-supplied command (including requested position,
  button or scroll), the complete supplied snapshot, request and target,
  requested/observed revision, action kind, and
  `Outcome::Accepted(PointerEvent)` or `Outcome::Rejected(RejectReason)`.
- `PointerEvent`: assistant actor, target, expected revision, position and
  move/button/scroll action. It is merely a proposal, not an injected event.

`adapt` checks that command, snapshot and grant refer to the assistant
workspace; that capability ID, target window and action match the simulated
scope; that the revision is current; and that coordinates stay within valid
bounds. Button/scroll events use the snapshot's current pointer position, not
an arbitrary position supplied by the caller. Scroll values are capped to
±1200 per axis as a **placeholder experiment limit**, not a platform standard.
The adapter does not own or increment revisions and is pure: state changes and
actual routing belong to another component.

The function trusts the snapshot and grant passed to it. `CapabilityId` is a
public integer, so it is trivially forgeable, and the enum actor tag is not
identity. Real security requires the OS or broker to authenticate callers,
protect and revoke capabilities and enforce grants at dispatch. The router must
recheck authorization, target and revision atomically with actual application
of the event, otherwise state can change between check and use. The trace is
returned data, not a durable or tamper-proof audit record. It records the
adapter's proposal/rejection, not the outcome of a real dispatch. A future
system can redact or restrict trace contents as appropriate; this sketch does
not implement privacy, access control, persistence or redaction.

Button state transitions, key input/focus, multi-device routing, concurrent
modification, ownership, pointer capture, overlapping windows, coordinate
transforms, IPC, persistence and real MCP tools are intentionally not
implemented. `no_std` does not prove isolation, or physical hardware
compatibility; using this library in a freestanding binary requires a working
allocator. Integrating the adapter into the other pointer router needs explicit
type conversion and authority semantics, not a claim that the layers already
interoperate.

## Verification

`cargo check --offline --lib --target wasm32-unknown-unknown`, compilation of
test sources for the same target, and `cargo fmt --check` pass on the research
machine. Those are compilation/format checks only; they do not exercise the
adapter or establish AArch64 integration. The test suite covers allowed and
rejected inputs, revision mismatch, bounds, capability scope and extreme scroll
values. It has not yet been executed for this version.

Run behavioral tests inside a disposable VM with Rust and Cargo:

```sh
cargo test --offline
```

No test executable was run on the host for this component.
