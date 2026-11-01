# Running the integrated system guest

`workspace_dynamic_storage` is the current integration guest: it boots one
Rust image with two fixed EL0 tasks and two programs loaded from a restricted
ELF subset on a virtual disk, virtual devices, a graphical demonstration, MCP
and a synthetic workspace saved on a VirtIO disk. Programs must obey this
fixture's ABI or they can halt or hang the guest. It is not a general OS,
general application loader, or native Apple Silicon boot image. See its
[README](../experiments/workspace_dynamic_storage/README.md) for the precise
capabilities and limitations.

Build the guest for `aarch64-unknown-none-softfloat`, then run:

```sh
cd experiments/workspace_dynamic_storage
cargo build --release --locked
python3 launch_vm.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/workspace-dynamic-storage-demo \
  --display cocoa
```

Use `--display gtk` on a QEMU installation with GTK, or `--display none` for a
headless run. The launcher creates a private temporary socket directory and a
disposable virtual disk. It prints the socket path for local MCP access. The
guest's subset of MCP is unauthenticated and must not be exposed to a network or
untrusted local users. MCP can modify the ELF files the guest executes; use it
only for isolated experiments. The guest has no network or host directory sharing.

To retain the **synthetic** disk between runs, add `--create-image /path/new.raw`
on the first run. It refuses to overwrite an existing path. On later runs use
`--resume-image /path/new.raw`; QEMU will write to that image. The latter only
accepts a regular, non-symlink 1 MiB file. Do not use a physical disk or a file
containing personal data. Close the QEMU window or interrupt the launcher to stop.
The guest executes fixed tasks automatically; it does not yet provide a shell.
