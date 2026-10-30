# Dual-input router: a VM-only Rust prototype

This `no_std` Rust library models two separate workspaces, human and assistant,
with independent pointer positions, windows, focus and workspace revisions. A
small freestanding AArch64 guest exercises it with **synthetic data only** in a
QEMU VM and emits a structured, visualizable trace. There is no macOS window,
graphical compositor, live mouse, real MCP server, model, OS process isolation
or operating-system security enforcement in this experiment.

The [structured VM evidence](evidence/qemu-trace.json) records seven state
transitions and an acknowledgement. A display program could consume the
synthetic positions and focus values, but they are not live device input.

## Behavior and limits

`Router::route` receives an actor, a caller-provided **simulated** grant and a
typed event. Within this single-threaded prototype it checks actor/workspace,
grant scope, target window, position and expected workspace revision before
changing state. It rejects cross-workspace events, unauthorized windows,
out-of-range positions and stale workspace snapshots. Shared synthetic objects
carry a separate revision; a write based on an older revision yields a conflict
without overwriting the newer value. Click is a single semantic **primary**
click in the model, with a window containment check. It requires focus permission
if the click would change focus. Real key input, button-down, button-up,
scrolling, hit-testing through overlapping windows and drag capture are not
implemented; no adapter may silently translate those into this click.

Each actor has its own position and focus; a move in one workspace does not
move the other. The guest validates this, the denial cases and a version
conflict. The calls are executed sequentially in one VM thread; the conflict
simulates stale concurrent observations, not parallel process execution. There
is no separate working-copy merge or automatic AI conflict resolution here.

`Grant` values and actor identities are ordinary caller-constructed data. They
are not authenticated, cannot safely defend against an adversarial caller and
are not enforced by hardware or a kernel access-control service. The `&mut`
router ensures exclusive access in this single caller, not cross-process
atomicity. The proposed production design needs trusted session identity,
revocation, synchronized state updates and per-operation authorization.

## AI serial command contract (not an interactive server)

The library parses exactly one complete, bounded ASCII line of the form
`AI_MOVE <positive-request-id> <x> <y>\n`, then converts it to a typed event.
The caller supplies a fresh workspace revision, checked again by the router.
Unsupported wire commands such as `AI_CLICK` or `AI_SCROLL`, malformed frames,
zero request IDs and invalid numbers are rejected; positions are checked
against the workspace bounds. The guest uses a **hard-coded synthetic command**
to demonstrate this path and only after the state update prints
`AI_ACK 17 MOVED 45 60`. It does not read commands from a live serial channel.
The VM runner checks that acknowledgement and all complete trace records.

A future interactive bridge would need framing, request IDs, matching success
or failure acknowledgements, bounded queues, timeouts, cancellation, identity,
permission and revalidation. A byte stream or this parser is **not MCP**; a
real MCP server must implement the protocol and invoke only authorized actions.
No AI inference or MCP call has been made in this experiment.

## Scaling beyond two actors

The current `Actor` and `WorkspaceId` enums explicitly contain only human and
assistant. Additional human or AI participants and revocable rights are **not
implemented**. The intended evolution uses dynamically allocated `ActorId` and
`WorkspaceId` values, explicit session identity, a workspace registry, one
pointer/focus per actor and versioned scoped capabilities revocable at each
dispatch. Event queues and resource costs would need limits, fairness and
backpressure. Remote participants require authenticated sessions and conflict
handling at the shared object. A thousand participants is merely an example of
why fixed two-person enums are insufficient, not a measured or guaranteed
capacity. Scale, latency, revocation and concurrency remain to be tested.

## Build and run the test inside a VM

Requires Cargo, Rust's `aarch64-unknown-none-softfloat` target, Python 3 and
QEMU. There are no third-party Rust dependencies. Build on the host, but execute
the guest **only in QEMU**:

```sh
cargo build --locked --offline --release
python3 run_vm.py --qemu qemu-system-aarch64
```

Use `--image` to point to a separately built guest and `--report` to choose a
JSON output location. The runner uses QEMU `virt`, TCG software emulation, one
virtual CPU, 128 MiB RAM, serial output, no guest network and no disk. It
terminates the guest after a final marker or timeout. The guest checks EL1 and
disabled MMU/data cache before initialising BSS; its 128 KiB bump allocator never
reclaims memory and is suitable only for this bounded one-shot fixture.

No test of the project code ran on the host. The isolated toolchain and QEMU
setup used for this run are documented in [the initial ARM boot report](../arm_boot/README.md).
Neither the compiler, QEMU nor generated guest binaries are committed here.
