#!/usr/bin/env python3
"""Run the guest static arena scenario in an isolated QEMU guest."""
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
    if "PASS guest static arena frame scenario" not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    # Verify the serial record includes the actual rejected operations and
    # one success record per allocated frame, without logging memory addresses.
    lines = set(serial.splitlines())
    required = {
        "IO action=free frame=2 seed=none result=rejected-reserved",
        "IO action=free frame=6 seed=none result=rejected-outside",
        "IO action=free frame=1 seed=none result=rejected-double-free",
        "IO action=reallocate frame=1 seed=none result=ok",
        "IO action=write frame=1 seed=231 result=ok",
        "IO action=read-reserved frame=2 seed=165 result=ok",
    }
    required.update(f"IO action=allocate frame={i} seed=none result=ok" for i in (0, 1, 3, 4, 5))
    if not required.issubset(lines) or " result=error" in serial or "unexpected-exhaustion" in serial:
        raise RuntimeError("missing or failed frame operation logs: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "guest-owned synthetic static arena, actual in-guest byte accesses",
              "limitations": "single-thread guest static arena; no discovered RAM, MMU or native driver"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "memory-arena-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS guest static arena scenario in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
