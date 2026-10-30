# Rust MCP client for the dual-pointer VM

This is a local Rust **client and transport**, not an MCP server, model, host
mouse controller, authentication service or security boundary. It connects to
the experimental guest's QEMU serial device over a Unix socket and speaks its
MCP 2025-11-25 JSON-RPC subset. The guest is an independent program; successful
compilation of this client does not prove the guest runs or the integration
works. A client cannot move the actual macOS cursor through this interface.

## Operation

1. Open a local Unix socket and negotiate `initialize`, then send
   `notifications/initialized`.
2. Discover `move_ai` and `pointer_state` via `tools/list`.
3. Read the guest pointer state, request the supplied AI cursor coordinates,
   and read state again. A successful run checks both the move confirmation
   and the final guest-reported position.

Requests and successfully parsed responses are included in a structured JSON
trace. On an error, the JSON written to standard error includes prior traced
requests and the failure; malformed raw responses are not logged. The model
only handles bounded serial lines and applies socket read/write timeouts. The
client caps messages at 64 KiB; this particular guest has a tighter 512-byte input
limit, and the fixed requests sent by this client are below that limit. The
socket path must identify a Unix socket, not an ordinary file. That check is
not authentication: any process with socket access may act as the peer. Use
only an isolated QEMU guest and a private socket directory with restrictive
permissions. No real pointer input, personal content or host system control is
provided. No component may treat this example as a permissions mechanism.

The guest limits the AI cursor to its right-hand region. This client checks the
same limits, verifies response IDs, protocol version and `isError`, and refuses
incompatible replies. It does not implement every MCP method, every valid MCP
response shape, reconnect/resume, arbitrary tool discovery, capabilities, or
concurrent requests. The current text-state parsing depends on the experimental
guest's `human=X,Y; ai=X,Y` format. It is an integration demonstrator, not a
general MCP SDK or evidence of full protocol conformance.

## Observed VM exchange

The [reproducible VM runner](../dual_pointer_display/README.md) completed on
the research Mac with QEMU TCG and retained its full
[synthetic QMP and MCP evidence](evidence/reproducible-vm-run.json). It
first verified that an out-of-bounds MCP move to `(399, 260)` was rejected
with `isError: true` and that the guest reported unchanged positions. It then
injected a synthetic tablet x=32767 event and observed the guest report
human x=390, then injected the standard synthetic human VirtIO tablet event
and captured the QMP responses. The client negotiated MCP, discovered the two tools and moved the
assistant cursor. The guest reported human pointer `(223, 390)` and
assistant pointer `(560, 330)` before the MCP call; afterward it reported
`(223, 390)` and `(650, 260)` respectively. The client checked the move reply
and post-move state. This run uses the guest connected to the `dual_input`
router: both pointer movement paths update its separate workspace cursor
states, which are read back by the display and `pointer_state`. The human
click indicator is a visual effect and is not a routed click or focus. Guest
grants are locally constructed and forgeable; they provide no authentication
or security isolation. The runner also recorded guest/client image hashes and
before/after framebuffer hashes. Different hashes establish a framebuffer
change, not the geometry or cause of that change; the generated raw
framebuffers themselves are not stored in this repository.

The JSON also preserves the previous automated run under
`previous_automated_evidence`; older evidence, including the pre-router run,
is retained in its nested records.
An earlier [synthetic exchange](evidence/mcp-smoke.json) used an earlier image.
The [manual exchange on the polished image](evidence/mcp-polished-smoke.json)
includes an operator-reported QMP acknowledgement and a hash taken after
startup; it did not retain an independent QMP transcript. All these examples
demonstrate guest-reported state in a virtual demo, not independent OS input
security, real OS workspaces or a full AI agent.

## Build and connect

Rust/Cargo, a running VM guest, and a local QEMU serial socket are required.
The [guest experiment](../dual_pointer_display/) supplies the server; its
setup and present status are documented separately. Compile on the developer
machine; all interaction with the experimental OS runs in the VM:

```sh
cargo build --release --locked
cargo run --release --locked -- /path/to/private/vm-serial.sock 550 250
```

The arguments are socket path, X and Y. Starting the command attempts the
specified movement; choose the coordinates intentionally.
Dependencies are pinned in `Cargo.lock`, but are not vendored. The program
logs requests and replies, so do not feed it secrets; the demo is designed for
synthetic positions only. Socket data can be incomplete on a timeout or peer
failure; an error does not prove whether a previously issued move took effect.
Do not blindly retry non-idempotent future actions.
