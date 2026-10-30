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
        time.sleep(0.3)
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
                                stress_request={"jsonrpc":"2.0","id":119,"method":"tools/call",
                                                "params":{"name":"exercise_human_moves","arguments":{}}}
                                stress_reply=rpc_call(serial_stream,stress_request)
                                stress_result=stress_reply.get("result",{})
                                if stress_result.get("isError") is not False:
                                    raise RuntimeError("guest bounded movement stress failed")
                                stress_data=json.loads(stress_result["content"][0]["text"])
                                if stress_data.get("moves")!=1101 or (stress_data.get("x"),stress_data.get("y"))!=(200,180) or stress_data.get("ring")!=64 or stress_data.get("overwritten",0)<=0 or stress_data.get("motion_allocations")!=0:
                                    raise RuntimeError("bounded movement evidence incorrect: "+str(stress_data))
                                # Check the rightmost synthetic tablet input before the standard scenario.
                                edge_event = {"execute": "input-send-event", "arguments": {"events": [
                                    {"type": "abs", "data": {"axis": "x", "value": 32767}},
                                ]}}
                                edge_result = qmp_command(qmp_stream, edge_event)
                                edge_poll = []
                                for request_id in range(4, 16):
                                    req, res, current = state(serial_stream, request_id)
                                    edge_poll.append({"request": req, "response": res})
                                    if current == "human=390,180; ai=560,330":
                                        break
                                    time.sleep(0.3)
                                else:
                                    raise TimeoutError("guest did not clamp the human pointer at x=390")
                                event = {"execute": "input-send-event", "arguments": {"events": [
                                    {"type": "abs", "data": {"axis": "x", "value": 18000}},
                                    {"type": "abs", "data": {"axis": "y", "value": 21000}},
                                ]}}
                                event_result = qmp_command(qmp_stream, event)
                                poll_trace = []
                                for request_id in range(16, 28):
                                    req, res, current = state(serial_stream, request_id)
                                    poll_trace.append({"request": req, "response": res})
                                    if "human=223,390" in current:
                                        break
                                    time.sleep(0.3)
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
                        # A new MCP session drives the actual bridge in this same guest.
                        doc_trace = []
                        with connect(serial, process) as doc_socket:
                            with doc_socket.makefile("rwb", buffering=0) as doc_stream:
                                init = {"jsonrpc":"2.0","id":100,"method":"initialize","params":
                                        {"protocolVersion":"2025-11-25","capabilities":{},
                                         "clientInfo":{"name":"vm-document-probe","version":"0.1"}}}
                                init_reply = rpc_call(doc_stream, init)
                                if init_reply.get("result", {}).get("protocolVersion") != "2025-11-25":
                                    raise RuntimeError("document probe could not initialize MCP")
                                doc_trace.append({"request":init,"response":init_reply})
                                doc_stream.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
                                doc_trace.append({"notification":{"jsonrpc":"2.0","method":"notifications/initialized"}})
                                list_request={"jsonrpc":"2.0","id":113,"method":"tools/list","params":{}}
                                list_response=rpc_call(doc_stream,list_request)
                                doc_trace.append({"request":list_request,"response":list_response})
                                tools={item["name"]:item for item in list_response.get("result",{}).get("tools",[])}
                                for name in ("document_edit","document_view"):
                                    properties=tools[name]["inputSchema"]["properties"]
                                    if properties["actor"].get("enum") != ["human","assistant"] or properties["space"].get("enum") != ["human-space","assistant-space"] or "32 UTF-8 bytes" not in properties["key"].get("description",""):
                                        raise RuntimeError("invalid document tool schema")
                                if "64 UTF-8 bytes" not in tools["document_edit"]["inputSchema"]["properties"]["value"].get("description",""):
                                    raise RuntimeError("invalid document value schema")
                                def doc_call(request_id, tool, arguments, rejected=False):
                                    request={"jsonrpc":"2.0","id":request_id,"method":"tools/call",
                                             "params":{"name":tool,"arguments":arguments}}
                                    response=rpc_call(doc_stream,request)
                                    doc_trace.append({"request":request,"response":response})
                                    result=response.get("result",{})
                                    if result.get("isError") is not rejected:
                                        raise RuntimeError("unexpected document tool outcome: "+str(response))
                                    if rejected:
                                        return None
                                    content=result.get("content",[])
                                    if len(content)!=1 or content[0].get("type")!="text":
                                        raise RuntimeError("invalid document response: "+str(response))
                                    return json.loads(content[0]["text"])
                                remote_before=doc_call(114,"remote_state",{})
                                if remote_before != {"actor":"remote","x":30,"y":40}:
                                    raise RuntimeError("unexpected initial third actor")
                                remote_move=doc_call(115,"move_remote",{"x":70,"y":90})
                                if remote_move != {"actor":"remote","x":70,"y":90}:
                                    raise RuntimeError("third actor move failed")
                                doc_call(116,"move_remote",{"x":800,"y":90},rejected=True)
                                if doc_call(117,"remote_state",{}) != remote_move:
                                    raise RuntimeError("rejected third actor move changed state")
                                req,res,positions=state(doc_stream,118)
                                doc_trace.append({"request":req,"response":res})
                                if positions != "human=223,390; ai=650,260":
                                    raise RuntimeError("third actor move changed visible actors")
                                h={"actor":"human","space":"human-space"}
                                a={"actor":"assistant","space":"assistant-space"}
                                doc_call(101,"document_edit",{**h,"key":"title","value":"human-version"})
                                doc_call(102,"document_edit",{**a,"key":"status","value":"assistant-ready"})
                                hv=doc_call(103,"document_view",{**h,"key":"status"})
                                av=doc_call(104,"document_view",{**a,"key":"title"})
                                if hv.get("value") is not None or av.get("value") != "base":
                                    raise RuntimeError("document copies not independent")
                                compatible=doc_call(105,"document_merge",{})
                                fields={field["key"]:field for field in compatible["fields"]}
                                if compatible["conflicts"] or compatible["materializable"] is not True or fields.get("title") != {"key":"title","value":"human-version","origin":"human"} or fields.get("status") != {"key":"status","value":"assistant-ready","origin":"assistant"}:
                                    raise RuntimeError("compatible merge or origins incorrect")
                                doc_call(106,"document_edit",{"actor":"assistant","space":"human-space","key":"title","value":"intrusion"},rejected=True)
                                human_after_reject=doc_call(111,"document_view",{**h,"key":"title"})
                                assistant_after_reject=doc_call(112,"document_view",{**a,"key":"title"})
                                if human_after_reject.get("value") != "human-version" or assistant_after_reject.get("value") != "base":
                                    raise RuntimeError("cross-space rejection changed a document copy")
                                doc_call(107,"document_edit",{**a,"key":"title","value":"assistant-version"})
                                conflict=doc_call(108,"document_merge",{})
                                if conflict["materializable"] is not False or conflict["conflicts"] != [{"key":"title","base":"base","human":"human-version","assistant":"assistant-version"}]:
                                    raise RuntimeError("conflict versions were not retained")
                                doc_call(109,"document_edit",{**h,"key":"é"*17,"value":"too-long-key"},rejected=True)
                                after_rejected=doc_call(110,"document_view",{**h,"key":"title"})
                                if after_rejected.get("value") != "human-version":
                                    raise RuntimeError("rejected edit changed the human copy")
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
                                              "bounded_movement_stress": {"request":stress_request,"response":stress_reply},
                                              "edge_input": {"request": edge_event, "response": edge_result,
                                                             "input_poll": edge_poll},
                                              "input_poll": poll_trace},
                                    "mcp_client": trace,
                                    "document_mcp_trace": doc_trace,
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
