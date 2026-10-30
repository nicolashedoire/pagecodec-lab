#!/usr/bin/env python3
"""Run the integrated MMU, EL0/SVC and controlled EL1 timer scenario."""
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
    if "PASS integrated MMU EL0 SVC timer scenario" not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    expected = {
        "BOOT el=1 pages=4k image=bounded uart=one-page tables=aligned",
        "MMU enabled=yes kernel=privileged user_code=ro user_stack=rw uart_gic=privileged",
        "CHECK at_el0 kernel_read=denied kernel_write=denied user_code=read_only user_stack=rw zeroed=yes",
        "ABORT el0_read_kernel=denied ec=0x24 dfsc=0x0f far=kernel elr=load count=1",
        "SVC first=0x1234 response=0x99 returned_to_el0=yes final=0x5678",
        "CHECK el0_sp=dedicated spsr=el0t kernel_data=unchanged user_stack=preserved",
        "TIMER irq=ppi27 count=1 interrupted=el1_wait resumed=yes disabled=yes gic=inactive",
    }
    if not expected.issubset(set(serial.splitlines())):
        raise RuntimeError("missing translation observations: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "synthetic EL0 SVC/protection and a subsequent EL1 virtual-timer IRQ",
              "limitations": "single-CPU single-address-space demo; no EL0 preemption, general process isolation or native Apple support"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "kernel-integrated-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS integrated MMU EL0 SVC timer scenario in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
