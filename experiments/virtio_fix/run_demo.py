#!/usr/bin/env python3
"""Exercise the patched RNG constructor against synthetic and QEMU devices."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time


def run(qemu, image, mode, fixture):
    command = [qemu, "-M", "virt,virtualization=off", "-accel", "tcg", "-cpu", "cortex-a57",
               "-smp", "1", "-m", "128M", "-nodefaults", "-display", "none", "-nic", "none",
               "-monitor", "none", "-serial", "stdio", "-no-reboot", "-kernel", image,
               "-global", "virtio-mmio.force-legacy=" + ("true" if mode == "legacy" else "false")]
    if fixture is not None:
        command += ["-object", f"rng-random,id=fixture,filename={fixture}", "-device", "virtio-rng-device,rng=fixture"]
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    data = bytearray()
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if selector.select(0.2):
                    data.extend(os.read(process.stdout.fileno(), 4096))
                    if any(line.startswith((b"PASS ", b"FAIL ")) for line in data.split(b"\n")[:-1]): break
                if process.poll() is not None: raise RuntimeError("QEMU exited: " + data.decode(errors="replace"))
            else: raise TimeoutError("guest did not finish: " + data.decode(errors="replace"))
    finally:
        process.terminate()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait()
    serial = data.decode("ascii", errors="replace").replace("\r", "")
    expected = {"modern": "QEMU modern rng_init=ok deterministic_bytes=32\nPASS patched virtio rng scenario",
                "legacy": "QEMU legacy rng_init=ok deterministic_bytes=32\nPASS patched virtio rng scenario",
                "invalid": "FAIL QEMU rng bytes", "absent": "FAIL no QEMU rng"}[mode]
    mock = "MOCK rng_constructor=feature_error failed=yes queue_calls=0 dma=0 driver_ok=0"
    reset = "MOCK rng_constructor=needs_reset_error queue_calls=0 driver_ok=0"
    if mock not in serial or reset not in serial or expected not in serial: raise RuntimeError("unexpected " + mode + " output: " + serial)
    return {"scenario": mode, "serial": serial, "expected_observed": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", required=True)
    parser.add_argument("--guest", required=True)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    guest = Path(args.guest)
    image_hash = hashlib.sha256(guest.read_bytes()).hexdigest()
    version = subprocess.run([args.qemu, "--version"], capture_output=True, text=True, check=True, timeout=10).stdout.splitlines()[0]
    with tempfile.TemporaryDirectory(prefix="virtio-fix-") as tmp:
        valid, invalid = Path(tmp) / "valid", Path(tmp) / "invalid"
        valid.write_bytes(bytes(range(256)) * 16)
        invalid.write_bytes(bytes([255]) * 4096)
        trials = [run(args.qemu, str(guest), "modern", valid), run(args.qemu, str(guest), "legacy", valid),
                  run(args.qemu, str(guest), "invalid", invalid), run(args.qemu, str(guest), "absent", None)]
    if hashlib.sha256(guest.read_bytes()).hexdigest() != image_hash: raise RuntimeError("guest image changed during trials")
    result = {"guest_sha256": image_hash, "qemu_version": version, "trials": trials,
              "rejection_source": "simulated transport inside guest", "fixtures": "temporary synthetic bytes only",
              "limitations": "local 0.13.0 patch, only RNG constructor exercised; not a complete VirtIO audit"}
    destination = Path(args.output_dir); destination.mkdir(parents=True, exist_ok=True)
    (destination / "virtio-fix-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS mocked refusal and QEMU modern/legacy positive and negative cases")


if __name__ == "__main__":
    main()
