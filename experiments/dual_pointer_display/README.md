# Two pointers in an AArch64 QEMU guest

This is a small experimental bare-metal Rust guest, not a general-purpose OS or a macOS replacement. It draws two panels with the QEMU virtio GPU 2D device. A QEMU virtio tablet controls the human pointer; an experimental MCP server in the guest can move the AI pointer. The panels are graphical mockups: they are not isolated processes, real workspaces, windows, or applications.

The server supports a small subset of MCP 2025-11-25 over newline-delimited JSON-RPC on the guest PL011 serial port: `initialize`, `notifications/initialized`, `ping`, `tools/list`, and `tools/call`. The tools are `pointer_state` and `move_ai`. A successful move response is sent only after the frame has been redrawn and flushed. `move_ai` accepts integer `x` in 400–773 and `y` in 0–573. It does not implement clicks. The guest is intended only for an isolated demonstration: its serial endpoint has no authentication, and it implements neither access control nor security isolation.

The demo boots on QEMU's `virt` machine with a single AArch64 CPU and 128 MiB RAM, uses a fixed framebuffer, 512-byte message line and a simple non-reclaiming bump allocator. It accepts 64 requests or notifications per boot, including invalid or oversized lines, and then stops; restart the VM for a new session. This bound reduces memory exhaustion but does not make the guest robust against adversarial clients. It has no persistent storage, networking, host file sharing, keyboard, scheduler, model inference or functional MCP operations other than pointer inspection and movement. The separately developed `dual_input` router is not connected to this display. It relies on `virtio-drivers` 0.13.0, which does not verify a device's `FEATURES_OK` acceptance after negotiation; this experiment only demonstrates QEMU accepting its devices.

Text is rendered using antialiased glyphs generated from **Inter**, designed by Rasmus Andersson and distributed under the [SIL Open Font License 1.1](assets/OFL.txt). The unmodified variable font is included at `assets/Inter-variable.ttf` (SHA-256 `29160a80ff49ddcab2c97711247e08b1fab27a484a329ce8b813d820dc559031`); source: [Google Fonts, Inter](https://github.com/google/fonts/tree/main/ofl/inter). `assets/generate_font.py` generates the glyph cells with Pillow; the running guest has no Python dependency.

Compile the guest for `aarch64-unknown-none-softfloat` with `cargo build --release --locked`, then launch the resulting ELF, using QEMU's native Cocoa backend on macOS or another available backend:

```sh
DEMO_DIR=$(mktemp -d)
chmod 700 "$DEMO_DIR"
qemu-system-aarch64 -M virt,virtualization=off -accel tcg -cpu cortex-a57 \
  -smp 1 -m 128M -nodefaults -device virtio-gpu-device -device virtio-tablet-device \
  -display cocoa -nic none -monitor none -no-reboot \
  -chardev "socket,id=mcp,path=$DEMO_DIR/mcp.sock,server=on,wait=off" \
  -serial chardev:mcp -kernel target/aarch64-unknown-none-softfloat/release/dual-pointer-display
```

Use a private socket path and connect one MCP client at a time; filesystem permissions are not authentication. Only expose this unauthenticated socket locally for the duration of an isolated test. The client must initialize the MCP session, send `notifications/initialized`, then use the listed tools. A serial socket disconnect does not reset guest state. A restarted VM resets all state. Remove the empty temporary directory after QEMU exits.

## Repeat the end-to-end demonstration

Compile this guest and the separate [`vm_mcp_client`](../vm_mcp_client/) first. The Python standard-library harness runs QEMU, injects a **synthetic** tablet event using QMP, runs the compiled Rust client against the guest MCP server, verifies both pointer positions and saves a genuine QMP framebuffer dump and a JSON trace. It uses a temporary private socket directory, no network, disks or shared host directories. A host-side client and harness only transport commands; the guest code executes inside the VM.

```sh
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest /path/to/dual-pointer-display --client /path/to/vm-mcp-client \
  --output-dir /path/to/output
```

The default is headless; add `--cocoa --hold 30` on macOS to show the window for 30 seconds after the capture. `framebuffer-before.ppm` and `framebuffer.ppm` are before/after framebuffers exported by QEMU. The harness checks that their hashes differ; `evidence.json` records both hashes, QEMU version, binary hashes and the protocol exchange. This is neither a benchmark nor evidence of human physical input. The script waits five seconds for boot by default; `--startup-wait` adjusts this for slower machines. It terminates its QEMU instance after the demo. Do not rebuild the supplied binaries while the demo runs.
