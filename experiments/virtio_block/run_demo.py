#!/usr/bin/env python3
"""Run the block guest with freshly created, disposable synthetic images only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time


def digest(data):
    return hashlib.sha256(data).hexdigest()


def trial(qemu, guest, directory, readonly):
    image = Path(directory) / ("readonly.raw" if readonly else "writable.raw")
    # Exclusive creation inside a new private directory. The runner has no option
    # to attach an existing host image or block device.
    fd = os.open(image, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    try:
        os.ftruncate(fd, 1024 * 1024)
    finally:
        os.close(fd)
    blocks = [
        {"driver": "file", "filename": str(image), "node-name": "file0", "read-only": readonly},
        {"driver": "raw", "file": "file0", "node-name": "disk0", "read-only": readonly},
    ]
    command = [qemu, "-M", "virt,virtualization=off", "-accel", "tcg", "-cpu", "cortex-a57",
               "-smp", "1", "-m", "128M", "-nodefaults", "-display", "none", "-nic", "none",
               "-monitor", "none", "-serial", "stdio", "-no-reboot", "-kernel", guest,
               "-blockdev", json.dumps(blocks[0]), "-blockdev", json.dumps(blocks[1]),
               "-device", "virtio-blk-device,drive=disk0"]
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    received = bytearray()
    try:
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        end = time.monotonic() + 30
        while time.monotonic() < end:
            if process.poll() is not None:
                raise RuntimeError("QEMU exited early: " + process.stderr.read().decode(errors="replace"))
            if selector.select(0.2):
                data = os.read(process.stdout.fileno(), 4096)
                received.extend(data)
                if any(line.startswith((b"PASS ", b"FAIL ")) for line in received.split(b"\n")[:-1]):
                    break
        else:
            raise TimeoutError("guest did not finish; serial=" + received.decode(errors="replace"))
    finally:
        process.terminate()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    serial = received.decode("ascii", errors="replace").replace("\r", "")
    raw = image.read_bytes()
    expected = bytearray(1024 * 1024)
    if not readonly:
        expected[4 * 512:6 * 512] = bytes((i * 37 + 11) % 256 for i in range(1024))
        if "PASS synthetic virtio block read write flush" not in serial:
            raise RuntimeError("guest failed writable fixture: " + serial)
    elif "FAIL write" not in serial:
        raise RuntimeError("guest did not reject read-only fixture: " + serial)
    if raw != expected:
        raise RuntimeError("fixture contents do not match expected bytes")
    return {"readonly": readonly, "serial": serial, "disk_sha256": digest(raw),
            "disk_bytes": len(raw), "expected_contents_verified": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", required=True)
    parser.add_argument("--guest", required=True)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    output = Path(args.output_dir)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="virtio-block-lab-") as directory:
        writable = trial(args.qemu, args.guest, directory, False)
        readonly = trial(args.qemu, args.guest, directory, True)
    evidence = {"guest_sha256": digest(Path(args.guest).read_bytes()),
                "fixture": "freshly created disposable synthetic 1 MiB images",
                "trials": [writable, readonly], "raw_host_block_devices_attached": False,
                "host_storage": "temporary disposable image files on the host filesystem"}
    (output / "virtio-block-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print("PASS writable read/write/flush and read-only rejection; evidence saved")


if __name__ == "__main__":
    main()
