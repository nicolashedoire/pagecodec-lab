#!/usr/bin/env python3
"""Run the synthetic fixed-weight inference in an isolated QEMU guest."""
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
    guest_hash=hashlib.sha256(Path(args.guest).read_bytes()).hexdigest()
    version=subprocess.run([args.qemu,"--version"],capture_output=True,text=True,check=True,timeout=10).stdout.splitlines()[0]
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
    expected={"CASE input=0,0,0 hidden=0,20,0,30 scores=-40,30",
              "CASE input=100,100,100 hidden=150,220,400,30 scores=460,880",
              "CASE input=70,20,10 hidden=130,20,80,0 scores=330,30",
              "CASE input=100,0,0 hidden=250,0,100,0 scores=610,-70",
              "CHECK repeat=deterministic invalid_inputs=rejected corner_bounds=ok control=none",
              "PASS synthetic integer neural inference"}
    if not expected.issubset(set(serial.splitlines())):
        raise RuntimeError("guest reported failure: " + serial)
    if hashlib.sha256(Path(args.guest).read_bytes()).hexdigest()!=guest_hash:
        raise RuntimeError("guest image changed during run")
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "synthetic numeric fixtures", "system_control": False}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "local-inference-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS synthetic integer neural inference in QEMU")


if __name__ == "__main__":
    main()
