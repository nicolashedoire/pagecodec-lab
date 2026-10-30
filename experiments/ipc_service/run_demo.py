#!/usr/bin/env python3
"""Run the bounded EL0 SVC dispatcher scenario in an isolated QEMU guest."""
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
    command = [args.qemu, "-M", "virt,virtualization=off,secure=off,gic-version=2", "-accel", "tcg", "-cpu", "cortex-a57",
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
    expected = [
        "MMU alias=same_va physical_pages=distinct asids=1,2 kernel=privileged",
        "CHECK counters=saturating_at_255",
        "CHECK requests=a:HELLO,PING b:OK,GO fifo=preserved",
        "REJECT pointer=page_cross,overflow,kernel size=17,short_buffer target=invalid mailbox_unchanged=yes",
        "REJECT full=a,b empty=a,a,b mailboxes_unchanged=yes",
        "CHECK generated_rounds=4 pointer_cases=36 size_cases=76 oracle=matched fifo_wrap=matched reject_mailboxes=unchanged",
        "CHECK a_send=10 a_receive=2 b_send=2 b_receive=10 switches=10 pages=expected",
        "PASS EL0 bounded IPC dispatcher",
    ]
    if serial.splitlines() != expected:
        raise RuntimeError("unexpected guest output: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "two synthetic EL0 actors exchange initial messages and four generated rounds",
              "limitations": "fixed single-CPU scenario; no general IPC, authentication, certified isolation, IOMMU, FP/SIMD or native Apple hardware"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "ipc-service-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS EL0 bounded IPC dispatcher; synthetic evidence saved")


if __name__ == "__main__":
    main()
