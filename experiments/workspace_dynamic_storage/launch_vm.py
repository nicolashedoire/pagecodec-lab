#!/usr/bin/env python3
"""Run the integrated guest with a private serial socket and virtual disk."""
import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile

DISK_SIZE = 1024 * 1024


def create_image(path):
    # Never overwrite an existing file, including a symlink or a physical device.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as image:
        image.truncate(DISK_SIZE)


def validate_image(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size != DISK_SIZE:
        raise ValueError("the virtual disk must be a regular 1 MiB image, not a device or symlink")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", required=True, help="path to qemu-system-aarch64")
    parser.add_argument("--guest", required=True, type=Path, help="compiled guest ELF")
    parser.add_argument("--display", choices=("cocoa", "gtk", "none"), default="none")
    disk = parser.add_mutually_exclusive_group()
    disk.add_argument("--create-image", type=Path, help="create a new persistent image; fails if it exists")
    disk.add_argument("--resume-image", type=Path, help="reuse a previously created image")
    args = parser.parse_args()
    guest = args.guest.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="unified-vm-") as runtime:
        # The guest's MCP serial protocol is unauthenticated; the socket directory
        # is private and no network or host directory is exposed to the guest.
        runtime = Path(runtime)
        os.chmod(runtime, 0o700)
        if args.create_image:
            image = args.create_image.absolute()
            create_image(image)
        elif args.resume_image:
            image = args.resume_image.absolute()
            validate_image(image)
        else:
            image = runtime / "scratch.raw"
            create_image(image)
        serial = runtime / "mcp.sock"
        file_node = {"driver": "file", "filename": str(image), "node-name": "file0"}
        raw_node = {"driver": "raw", "file": "file0", "node-name": "disk0"}
        cmd = [args.qemu, "-M", "virt,virtualization=off", "-accel", "tcg",
               "-cpu", "cortex-a57", "-smp", "1", "-m", "128M", "-nodefaults",
               "-display", args.display, "-nic", "none", "-monitor", "none", "-no-reboot",
               "-device", "virtio-gpu-device", "-device", "virtio-tablet-device",
               "-chardev", f"socket,id=mcp,path={serial},server=on,wait=off",
               "-serial", "chardev:mcp", "-kernel", str(guest),
               "-global", "virtio-mmio.force-legacy=false",
               "-blockdev", json.dumps(file_node), "-blockdev", json.dumps(raw_node),
               "-device", "virtio-blk-device,drive=disk0"]
        print(f"MCP socket: {serial}", flush=True)
        print(f"Virtual disk: {image} ({'temporary' if not (args.create_image or args.resume_image) else 'persistent'})", flush=True)
        print("The guest runs its fixed synthetic scenario; it is not a general desktop.", flush=True)
        try:
            result = subprocess.run(cmd, check=False)
        except KeyboardInterrupt:
            return 130
        return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
