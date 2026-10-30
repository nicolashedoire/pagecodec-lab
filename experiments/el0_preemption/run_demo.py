#!/usr/bin/env python3
"""Run the controlled timer-driven task alternation scenario in an isolated QEMU guest."""
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
    if "PASS timer preemption of two EL0 loops" not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    lines = serial.splitlines()
    if "BOOT el0_tasks=2 stacks=separate timer=ppi27 yields=0" not in lines:
        raise RuntimeError("missing boot marker: " + serial)
    if "CHECK el0_frames=two gpr=x0-x30_except_counter user_sp=separate kernel_irq_stack=bounded" not in lines:
        raise RuntimeError("missing context checks: " + serial)
    if "MMU el0_code=ro el0_stacks=rw kernel_data_uart_gic=privileged" not in lines or "CHECK el0_permissions=translation_queries timer=disabled gic=inactive terminal_snapshot=residual" not in lines:
        raise RuntimeError("missing MMU or terminal checks: " + serial)
    irq_lines = [line for line in lines if line.startswith("IRQ tick=")]
    if len(irq_lines) != 8:
        raise RuntimeError("expected eight actual IRQ observations: " + serial)
    for i, line in enumerate(irq_lines):
        expected = "IRQ tick=" + str(i+1) + " el0_task=" + str(i%2) + " count="
        suffix = " next=" + (str(1-i%2) if i<7 else "finish")
        if not line.startswith(expected) or not line.endswith(suffix):
            raise RuntimeError("unexpected IRQ ordering: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "eight guest virtual-timer IRQs selecting two EL0 assembly loops from a fixed ready ring",
              "limitations": "single-CPU QEMU integer-only test; no blocking, general process isolation, FP/SIMD or native Apple hardware"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "el0-preemption-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS timer preemption of EL0 loops in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
