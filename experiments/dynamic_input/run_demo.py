#!/usr/bin/env python3
"""Run the synthetic dynamic input scenario in an isolated QEMU guest."""
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
    if "PASS synthetic dynamic input routing" not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    lines = set(serial.splitlines())
    required = {
        "IO actor=human space=human-space x=10 y=20 result=accepted",
        "IO actor=assistant space=assistant-space x=20 y=30 result=accepted",
        "IO actor=remote space=remote-space x=30 y=40 result=accepted",
        "IO actor=assistant space=human-space x=5 y=5 result=wrong-space",
        "IO actor=human space=human-space x=100 y=20 result=out-of-bounds",
        "IO actor=unknown space=human-space x=5 y=5 result=unknown-actor",
        "IO actor=human space=unknown-space x=5 y=5 result=unknown-space",
        "IO actor=human space=invalid-space x=0 y=0 result=unknown-space",
        "IO actor=human space=human-space x=99 y=99 result=accepted",
        "IO actor=human space=human-space x=-1 y=0 result=out-of-bounds",
        "IO actor=human space=human-space x=0 y=100 result=out-of-bounds",
        "CHECK actors=19 spaces=3 independent_pointers=yes rejected_attempts=logged",
    }
    if not required.issubset(lines):
        raise RuntimeError("missing synthetic input records: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "synthetic actor, workspace and pointer events",
              "limitations": "unauthenticated in-memory state model; not connected to graphical guest"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "dynamic-input-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS dynamic input scenario in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
