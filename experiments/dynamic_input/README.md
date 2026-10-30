# Dynamic multi-actor pointer routing sketch

This `no_std` Rust library connects [`actor_core`](../actor_core/) to a single-threaded logical pointer router. The caller registers workspace IDs with half-open local bounds, creates actor IDs and kinds, assigns each actor to a workspace and submits pointer moves. The router verifies that the workspace and actor exist, that the actor is assigned to that workspace, and that the position is inside that workspace. It retains a structured in-memory trace for every submitted move, including rejection reasons. Each actor retains its own pointer even when several actors share a workspace.

There is no fixed actor-count limit in the API; available memory and allocator behavior limit the number of actors and trace entries. The example guest uses a finite 128 KiB bump heap without reclamation; out-of-memory handling, actor removal, bounded log retention, backpressure, synchronization and persistence are not implemented. Registering an actor, its kind and its workspace is separate from moving a pointer, and setup/assignment attempts are not included in the movement trace. Reassigning an actor clears its pointer through `actor_core`. This experiment supports moves only: clicks, focus, scroll, keyboards and window surfaces are not implemented.

**Labels are not identity or authorization.** Any caller with mutable access can create actors or move on their behalf. A simulated actor/space binding prevents an accidental mismatched-space event but cannot authenticate a human, an AI or a remote participant, protect data, or provide process isolation. The trace is caller-supplied data held in memory; it is not a durable or tamper-proof audit log. It must not be used with sensitive real inputs as-is. The router is **not yet connected** to the graphical guest, its tablet, MCP server or the separate document bridge; it does not replace their current two-actor router.

## VM scenario

The guest configures three 100×100 spaces and actors labeled human, assistant and remote. It checks independent initial pointer states, rejection of wrong-space, out-of-bounds, unknown-space and unknown-actor moves, unchanged state after rejections, half-open boundary behavior, and failed creation of an invalid space. It then creates 16 additional actors sharing a space, moves each, and verifies the existing remote pointer remains unchanged. The 19 actors in this test are a synthetic scenario, not a capacity guarantee.

The guest prints the **actual router trace** after the scenario, including synthetic actor and workspace labels, coordinates and accepted/rejected outcomes. The runner checks representative log records as well as the guest's final success line. The evidence applies to this single QEMU configuration; it does not prove independent desktop focus, security or real hardware support.

With Rust, target `aarch64-unknown-none-softfloat`, Python 3 and QEMU installed, run from this directory:

```sh
cargo build --release --locked --offline --features qemu-demo
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/dynamic-input-demo \
  --output-dir /path/to/output
```

The host Python harness launches an isolated QEMU `virt` guest without network, disks or shared host directories and collects serial output, guest ELF hash and QEMU version. See the [synthetic QEMU evidence](evidence/dynamic-input-evidence.json). All component execution tests are performed in the VM.
