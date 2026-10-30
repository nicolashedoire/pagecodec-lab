#!/usr/bin/env python3
"""Launch the synthetic actor scenario in an isolated QEMU guest."""
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
    command = [args.qemu, "-M", "virt,virtualization=off", "-accel", "tcg", "-cpu", "cortex-a57",
               "-smp", "1", "-m", "128M", "-nodefaults", "-display", "none", "-nic", "none",
               "-monitor", "none", "-serial", "stdio", "-no-reboot", "-kernel", args.guest]
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    output = bytearray()
    try:
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError("QEMU exited: " + process.stderr.read().decode(errors="replace"))
            if selector.select(0.2):
                output.extend(os.read(process.stdout.fileno(), 4096))
                if any(line.startswith((b"PASS ", b"FAIL ")) for line in output.split(b"\n")[:-1]):
                    break
        else:
            raise TimeoutError("guest did not finish")
    finally:
        process.terminate()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    serial = output.decode("ascii").replace("\r", "")
    if "PASS actor core synthetic scenario" not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    result = {"guest_sha256": hashlib.sha256(Path(args.guest).read_bytes()).hexdigest(),
              "serial": serial, "input": "synthetic fixture", "security_isolation": False}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "actor-core-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS actor scenario in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
