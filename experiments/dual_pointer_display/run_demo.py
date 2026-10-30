#!/usr/bin/env python3
"""Launch the isolated QEMU guest, exercise MCP and save real VM evidence.

This is a host-side transport harness; the guest code executes only inside QEMU.
"""
import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def connect(path, process):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("QEMU exited before its socket became available")
        stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            stream.connect(str(path))
            stream.settimeout(15)
            return stream
        except (FileNotFoundError, ConnectionRefusedError):
            stream.close()
            time.sleep(0.1)
    raise TimeoutError("QEMU socket did not become available")


def qmp_command(stream, request):
    stream.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
    while True:
        line = stream.readline()
        if not line:
            raise RuntimeError("QMP disconnected")
        response = json.loads(line)
        if "return" in response:
            return response
        if "error" in response:
            raise RuntimeError("QMP failed: " + str(response["error"]))


def wait_for_guest_frame(stream, filename, process):
    """Wait for the guest's 800x600 non-uniform display, not the QEMU splash."""
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("QEMU exited while waiting for guest display")
        try:
            qmp_command(stream, {"execute": "screendump", "arguments": {"filename": str(filename)}})
            data = filename.read_bytes()
            # PPM P6 as written by QEMU: header and raw RGB bytes. A small sample
            # detects a drawn surface rather than a single-colour startup frame.
            parts = data.split(b"\n", 3)
            if (len(parts) == 4 and parts[0] == b"P6" and parts[1] == b"800 600"
                    and parts[2] == b"255" and len(set(parts[3][::997])) > 1):
                return
        except (OSError, RuntimeError):
            # The display may not be initialized yet. Retry until the deadline.
            pass
        time.sleep(0.2)
    raise TimeoutError("guest did not display an initialized 800x600 framebuffer")


def rpc_call(stream, request):
    stream.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
    line = stream.readline()
    if not line:
        raise RuntimeError("guest MCP socket disconnected")
    response = json.loads(line)
    if response.get("jsonrpc") != "2.0" or response.get("id") != request["id"]:
        raise RuntimeError("unexpected MCP response: " + str(response))
    if "error" in response:
        raise RuntimeError("guest MCP error: " + str(response["error"]))
    return response


def state(stream, request_id):
    request = {"jsonrpc": "2.0", "id": request_id, "method": "tools/call", "params":
               {"name": "pointer_state", "arguments": {}}}
    response = rpc_call(stream, request)
    result = response.get("result", {})
    content = result.get("content", [])
    if result.get("isError") is not False or len(content) != 1 or content[0].get("type") != "text":
        raise RuntimeError("unexpected pointer_state response: " + str(response))
    return request, response, content[0].get("text", "")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", required=True, help="qemu-system-aarch64 executable")
    parser.add_argument("--guest", required=True, help="compiled AArch64 guest ELF")
    parser.add_argument("--client", required=True, help="compiled Rust MCP client")
    parser.add_argument("--output-dir", required=True, help="directory for screenshot and trace")
    parser.add_argument("--cocoa", action="store_true", help="show a Cocoa window on macOS")
    parser.add_argument("--hold", type=float, default=0, help="seconds to keep the window open after capture")
    parser.add_argument("--startup-wait", type=float, default=0, help="optional minimum wait before checking readiness")
    args = parser.parse_args()
    if args.hold < 0 or args.startup_wait < 0:
        parser.error("wait values must be nonnegative")
    destination = Path(args.output_dir).resolve()
    destination.mkdir(parents=True, exist_ok=True)
    # Hash exactly the images we ask QEMU and the client to run; don't rebuild concurrently.
    guest_hash, client_hash = sha256(args.guest), sha256(args.client)
    with tempfile.TemporaryDirectory(prefix="dual-vm-") as directory:
        # TemporaryDirectory is mode 0700. Access is limited but not authenticated.
        serial, qmp = Path(directory) / "serial.sock", Path(directory) / "qmp.sock"
        command = [args.qemu, "-M", "virt,virtualization=off", "-accel", "tcg", "-cpu", "cortex-a57",
                   "-smp", "1", "-m", "128M", "-nodefaults", "-device", "virtio-gpu-device",
                   "-device", "virtio-tablet-device", "-display", "cocoa" if args.cocoa else "none",
                   "-nic", "none", "-chardev", f"socket,id=mcp,path={serial},server=on,wait=off",
                   "-serial", "chardev:mcp", "-monitor", "none", "-qmp", f"unix:{qmp},server=on,wait=off",
                   "-no-reboot", "-kernel", args.guest]
        with tempfile.TemporaryFile(mode="w+b") as qemu_log:
            process = subprocess.Popen(command, stdout=qemu_log, stderr=subprocess.STDOUT)
            try:
                with connect(qmp, process) as qmp_socket:
                    with qmp_socket.makefile("rwb", buffering=0) as qmp_stream:
                        greeting = json.loads(qmp_stream.readline())
                        if "QMP" not in greeting:
                            raise RuntimeError("invalid QMP greeting")
                        qmp_command(qmp_stream, {"execute": "qmp_capabilities"})
                        if args.startup_wait:
                            time.sleep(args.startup_wait)
                        wait_for_guest_frame(qmp_stream, Path(directory) / "ready.ppm", process)
                        # Probe and poll on one connection. The fixed guest's 64-message
                        # lifetime bounds the retry count and leaves room for the client.
                        with connect(serial, process) as serial_socket:
                            with serial_socket.makefile("rwb", buffering=0) as serial_stream:
                                initialize = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params":
                                              {"protocolVersion": "2025-11-25", "capabilities": {},
                                               "clientInfo": {"name": "vm-demo-probe", "version": "0.1"}}}
                                init_response = rpc_call(serial_stream, initialize)
                                if init_response.get("result", {}).get("protocolVersion") != "2025-11-25":
                                    raise RuntimeError("guest did not negotiate MCP protocol")
                                serial_stream.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
                                rejected = {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params":
                                            {"name": "move_ai", "arguments": {"x": 399, "y": 260}}}
                                rejection = rpc_call(serial_stream, rejected)
                                if rejection.get("result", {}).get("isError") is not True:
                                    raise RuntimeError("guest accepted out-of-bounds coordinates")
                                before_req, before_res, before_state = state(serial_stream, 3)
                                if "ai=560,330" not in before_state:
                                    raise RuntimeError("rejected move changed the AI pointer: " + before_state)
                                event = {"execute": "input-send-event", "arguments": {"events": [
                                    {"type": "abs", "data": {"axis": "x", "value": 18000}},
                                    {"type": "abs", "data": {"axis": "y", "value": 21000}},
                                ]}}
                                event_result = qmp_command(qmp_stream, event)
                                poll_trace = []
                                for request_id in range(4, 49):
                                    req, res, current = state(serial_stream, request_id)
                                    poll_trace.append({"request": req, "response": res})
                                    if "human=223,390" in current:
                                        break
                                    time.sleep(0.2)
                                else:
                                    raise TimeoutError("guest did not consume the synthetic tablet event")
                        before_image = destination / "framebuffer-before.ppm"
                        qmp_command(qmp_stream, {"execute": "screendump", "arguments": {"filename": str(before_image)}})
                        # The separate client handles MCP initialization, discovery, movement and verification.
                        try:
                            completed = subprocess.run([args.client, str(serial), "650", "260"], capture_output=True,
                                                       text=True, timeout=30, check=True)
                        except subprocess.CalledProcessError as error:
                            raise RuntimeError("MCP client failed: " + error.stderr + error.stdout) from error
                        trace = json.loads(completed.stdout)
                        result = trace.get("result", trace)
                        before, after = result.get("before", ""), result.get("after", "")
                        if "human=223,390" not in before or "human=223,390" not in after or "ai=650,260" not in after:
                            raise RuntimeError("unexpected guest pointer state: " + str((before, after)))
                        screenshot = destination / "framebuffer.ppm"
                        qmp_command(qmp_stream, {"execute": "screendump", "arguments": {"filename": str(screenshot)}})
                        before_hash, after_hash = sha256(before_image), sha256(screenshot)
                        if before_hash == after_hash:
                            raise RuntimeError("framebuffer did not change after MCP pointer move")
                        evidence = {"guest_sha256": guest_hash, "client_sha256": client_hash,
                                    "qemu_version": greeting["QMP"]["version"],
                                    "human_input": "synthetic QMP virtio-tablet event", "qmp_event": event,
                                    "qmp_response": event_result,
                                    "probe": {"initialization": {"request": initialize, "response": init_response},
                                              "out_of_bounds": {"request": rejected, "response": rejection},
                                              "after_rejection": {"request": before_req, "response": before_res},
                                              "input_poll": poll_trace},
                                    "mcp_client": trace,
                                    "framebuffer_before": {"file": before_image.name, "sha256": before_hash},
                                    "framebuffer_after": {"file": screenshot.name, "sha256": after_hash},
                                    "framebuffer_claim": "hashes demonstrate a change, not pointer geometry",
                                    "limitations": "experimental VM; no authentication, real workspaces or host OS control"}
                        (destination / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
                        print("Demo succeeded; saved before/after framebuffers and evidence.json")
                        if args.hold:
                            time.sleep(args.hold)
            except Exception:
                qemu_log.flush()
                qemu_log.seek(0)
                diagnostic = qemu_log.read().decode("utf-8", errors="replace")
                if diagnostic:
                    print("QEMU diagnostic output:\n" + diagnostic, file=__import__("sys").stderr)
                raise
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
