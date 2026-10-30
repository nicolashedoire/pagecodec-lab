# Synthetic interactive panels in a QEMU graphical guest

This experimental guest starts from a copy of [`dual_pointer_display`](../dual_pointer_display/) and uses the local [patched VirtIO driver](../virtio_fix/). It draws two panels with two synthetic buttons each, plus human, assistant and remote pointers. Buttons are colored by a per-actor **logical focus** and keep separate in-memory click counters. These are controls implemented directly in one guest framebuffer, not windows, applications, keyboard focus, protected processes or isolated workspaces. The demo pre-registers three actor labels; that is not an architectural actor limit.

The human pointer follows the guest VirtIO tablet. A primary-button press edge activates a button only if the human pointer is inside its half-open hit rectangle; release does not count as another click. An outside press changes the pressed indicator but not the focused button or click count. The assistant pointer moves with `move_ai`; `click_ai` activates the button under that pointer, or returns an error if it is not over an assistant button. `interaction_state` exposes the separate focus labels, click counters and human pressed state. A successful MCP click response is sent after redraw/flush. If a GPU flush fails after the state update, the guest returns an error and halts without rolling back that state.

These interactions are implemented directly by the demo's button state, not by the `dynamic_input` router's focus/click protocol. Actor labels, `click_ai` and the local serial endpoint are **not authenticated**. Anyone with access to that socket can request the assistant action. No host cursor, keyboard, files or real applications are controlled. The remote pointer remains a labeled visual pointer, not an additional application. The counts saturate rather than overflow; the finite non-reclaiming guest allocator and 64-message per-boot limit make this a short-lived demonstration, not a production input stack.

## QEMU evidence

The [modern-MMIO evidence](evidence/interaction-evidence.json) and [legacy-MMIO evidence](evidence/interaction-legacy-evidence.json) retain synthetic QMP input requests/responses, MCP requests/responses and before/after framebuffer hashes. Each scenario checks:

- An assistant click outside a button is rejected without changing interaction state.
- Synthetic QMP tablet motion and primary press target the human `ACTION` button; the release does not increment the counter a second time.
- An MCP move and click target the assistant `SELECT` button while the human focus and count remain unchanged.
- A second synthetic human click targets `SELECT` without changing assistant focus or count; a later outside click leaves both unchanged.
- With assistant focus already set, clicks at the left, right, top and bottom outside edges of the assistant `ACTION` rectangle are rejected without changing either actor's interaction state. Clicks on the included corner coordinates of both assistant buttons are accepted and counted. These are tests of the guest's half-open hitboxes, not physical input or general window hit-testing.
- A synthetic document edit and merge still work in the same guest. Framebuffer hashes differ across the human and assistant clicks. The hashes prove framebuffer changes, not exact button geometry or physical-human input.

The runner limits each poll to three attempts, keeping the successful worst-case sequence within the guest's 64-message limit; a slower run can fail its bounded wait rather than report an unobserved result. The runner tests one QEMU guest in each run. Its QMP tablet events are synthetic: they do not demonstrate an actual person's hardware input. The local patch checks modern VirtIO feature-negotiation acceptance, but these scenarios only exercise accepting QEMU devices, not a device refusing negotiation. There is no native Apple driver, OS integration, security isolation, comprehensive protocol compliance or performance claim.

The copied glyphs use **Inter**, designed by Rasmus Andersson under the [SIL Open Font License 1.1](assets/OFL.txt); source: [Google Fonts](https://github.com/google/fonts/tree/main/ofl/inter).

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3, from this directory:

```sh
cargo build --release --locked
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/graph-interaction-demo \
  --output-dir /path/to/output
# Optional: repeat with --transport legacy
```

The host harness starts an isolated QEMU VM without networking, disks or host directory sharing, sends synthetic QMP/MCP messages, and records serial replies, image hash, QEMU version and framebuffer hashes. The code under test runs in the VM. Do not rebuild the guest during a run; the runner checks the binary hash before and after execution.
