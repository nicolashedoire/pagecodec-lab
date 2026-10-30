#!/usr/bin/env python3
"""Run modern acceptance, absent device and legacy rejection in QEMU guests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time


def trial(qemu, guest, mode):
    command = [qemu, "-M", "virt,virtualization=off", "-accel", "tcg", "-cpu", "cortex-a57",
               "-smp", "1", "-m", "128M", "-nodefaults", "-display", "none", "-nic", "none",
               "-monitor", "none", "-serial", "stdio", "-no-reboot", "-kernel", guest,
               "-global", "virtio-mmio.force-legacy=" + ("true" if mode == "legacy" else "false")]
    if mode != "absent":
        command += ["-object", "rng-random,id=fixture,filename=/dev/zero", "-device", "virtio-rng-device,rng=fixture"]
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if selector.select(0.2):
                    output.extend(os.read(process.stdout.fileno(), 4096))
                    if any(line.startswith((b"PASS ", b"FAIL ")) for line in output.split(b"\n")[:-1]):
                        break
                if process.poll() is not None:
                    raise RuntimeError("QEMU exited: " + output.decode(errors="replace"))
            else:
                raise TimeoutError("guest did not finish: " + output.decode(errors="replace"))
    finally:
        process.terminate()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    serial = output.decode("ascii", errors="replace").replace("\r", "")
    expected = {"modern": "PASS virtio feature negotiation scenario", "absent": "FAIL no modern QEMU virtio rng",
                "legacy": "FAIL QEMU device legacy"}[mode]
    if expected not in serial or "MOCK checked_begin=rejected failed=yes queue_setup=no driver_ok=no" not in serial:
        raise RuntimeError("unexpected " + mode + " output: " + serial)
    if mode == "legacy" and "QEMU legacy checked_begin=unsupported" not in serial:
        raise RuntimeError("legacy did not reach checked_begin: " + serial)
    return {"scenario": mode, "serial": serial, "expected_observed": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", required=True)
    parser.add_argument("--guest", required=True)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    guest = Path(args.guest)
    guest_hash = hashlib.sha256(guest.read_bytes()).hexdigest()
    version = subprocess.run([args.qemu, "--version"], capture_output=True, text=True, check=True, timeout=10).stdout.splitlines()[0]
    trials = [trial(args.qemu, str(guest), mode) for mode in ("modern", "absent", "legacy")]
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed during trials")
    result = {"guest_sha256": guest_hash, "qemu_version": version, "trials": trials,
              "rejection_source": "simulated in-guest transport; QEMU device accepts features",
              "limitations": "prototype only; not a patched dependency or complete VirtIO driver"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "virtio-negotiation-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS simulated refusal, QEMU modern acceptance, absent and legacy cases")


if __name__ == "__main__":
    main()
