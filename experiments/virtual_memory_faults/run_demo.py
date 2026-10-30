#!/usr/bin/env python3
"""Run a controlled EL1 permission fault in an isolated QEMU guest."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", required=True)
    parser.add_argument("--guest", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--writable-control", action="store_true", help="expect the separate writable-control build")
    args = parser.parse_args()
    guest = Path(args.guest)
    guest_hash = hashlib.sha256(guest.read_bytes()).hexdigest()
    command = [args.qemu, "-M", "virt,virtualization=off", "-accel", "tcg", "-cpu", "cortex-a57",
               "-smp", "1", "-m", "128M", "-nodefaults", "-display", "none", "-nic", "none",
               "-monitor", "none", "-serial", "stdio", "-no-reboot", "-kernel", str(guest)]
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if selector.select(0.2):
                    data = os.read(process.stdout.fileno(), 4096)
                    output.extend(data)
                    if any(line.startswith((b"PASS ", b"FAIL ")) for line in output.split(b"\n")[:-1]):
                        break
                if process.poll() is not None:
                    raise RuntimeError("QEMU exited: " + output.decode(errors="replace"))
            else:
                raise TimeoutError("guest did not finish: " + output.decode(errors="replace"))
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    serial = output.decode("ascii", errors="replace").replace("\r", "")
    pass_line = "PASS writable permission control" if args.writable_control else "PASS controlled EL1 write permission fault"
    if pass_line not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    expected = {
        "BOOT el=1 pages=4k image=bounded uart=one-page tables=aligned",
        "MMU enabled=yes translation=identity text=ro-exec data=rw-xn uart=device-xn",
        "CHECK probe=read-write stack=translated uart=translated neighbor-pages=unmapped",
        "CHECK text-write=denied data-write=allowed el0-reads=denied xn=descriptor-bits",
    }
    if args.writable_control:
        expected.add("CONTROL writable_store=completed faults=0 other_words=unchanged")
    else:
        expected.update({"FAULT ec=0x25 wnr=1 dfsc=0x0f far=probe elr=faulting-store count=1",
                         "RECOVERY elr=next-instruction page=unchanged words=512"})
    if not expected.issubset(set(serial.splitlines())):
        raise RuntimeError("missing translation observations: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "one controlled guest write to a " + ("writable" if args.writable_control else "read-only") + " page",
              "limitations": "single-CPU EL1 fault demonstration; no process isolation or native Apple support"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    filename = "virtual-memory-control-evidence.json" if args.writable_control else "virtual-memory-faults-evidence.json"
    (destination / filename).write_text(json.dumps(result, indent=2) + "\n")
    print("PASS EL1 " + ("writable control" if args.writable_control else "permission fault") + " in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
