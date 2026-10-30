# Extensible actor and pointer state

`actor-core-sketch` is a Rust `no_std` library using `alloc`. It maintains logical actor IDs and kinds (human, assistant or remote), space IDs, each actor's assigned space and optional pointer position, and an in-memory event sequence. There is no API-defined maximum number of actors or spaces; capacity is limited by available memory. IDs must be nonempty and contain only lowercase ASCII letters, digits, `_` or `-`; actor and space IDs are distinct types.

The registry rejects duplicates, unknown IDs and pointer moves against a space other than the actor's currently assigned space. Failed operations do not add events. Assigning an actor to a space resets its pointer, even when assigning the same space, because coordinates must be set again. Coordinates are signed integers; the registry itself does not assign them a coordinate system or enforce display bounds. The connected graphical guest uses `dynamic_input` bounds in global screen coordinates. The registry supports multiple actors assigned to one space and retains a separate pointer per actor. The usual `move_pointer` records an in-memory, unbounded event. The separate `move_pointer_unlogged` updates state without appending an event, allowing a caller to supply its own bounded recording policy; it must not be treated as a complete audit history. Neither path is a durable audit log.

**This is not security isolation.** IDs are unauthenticated labels, `ActorKind` is caller supplied, and any caller with a mutable registry reference can request changes for any actor. There is no access control, secure broker, process isolation, networking, shared storage, compositor, user interface, concurrency control or authorization of the MCP/pointer demos. A separate QEMU graphical prototype uses this registry through `dynamic_input` for two rendered pointers and a third actor with no displayed cursor. Its pointer registry is separate from the document bridge registry. Its preallocated 64-record movement journal overwrites old entries, and a VM test exercises 1,101 synthetic moves followed by a separate QMP tablet event. A future system must enforce authenticated identity and permissions outside this model. It is a state model, not an OS scheduler or input driver.

## Integrate

A Rust guest or host program may depend on this crate as a path dependency and use `Registry`, `ActorId`, `SpaceId`, `ActorKind` and `Position`. A `no_std` consumer must supply a global allocator. Serialize or protect the registry externally if multiple execution contexts use it; the crate does not make concurrent access safe on its own.

## Reproduce the synthetic VM test

Install Rust with target `aarch64-unknown-none-softfloat`, QEMU AArch64, and Python 3. From this directory:

```sh
cargo build --release --locked --features qemu-demo
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/actor-core-demo \
  --output-dir /path/to/output
```

The guest runs in QEMU TCG without network, disks or host directory sharing. Python only launches QEMU and collects serial output. The guest uses a 128 KiB bump allocator for this one-shot synthetic demonstration, so its test capacity is finite; the library itself does not set an actor count limit. The test covers three kinds, 19 actors, three spaces, attribution of pointer events, independent state, invalid IDs, duplicates, unknown actors and spaces, rejected inter-space moves, and clearing the pointer on reassignment. It does not prove security, concurrency, hardware support or general correctness. See the [recorded synthetic output](evidence/actor-core-evidence.json).
