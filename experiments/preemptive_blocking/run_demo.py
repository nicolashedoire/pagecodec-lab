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
    if "PASS preemptive block wake idle scenario" not in serial:
        raise RuntimeError("guest reported failure: " + serial)
    lines = serial.splitlines()
    if "BOOT qemu-gicv2 timer=virtual-ppi27 tasks=2 idle=wfi yields=0" not in lines:
        raise RuntimeError("missing boot marker: " + serial)
    if "CHECK task_and_idle_frames=verified timer=disabled gic=inactive terminal_snapshot=residual" not in lines:
        raise RuntimeError("missing context checks: " + serial)
    if "CHECK empty_ready=idle_sentinel" not in lines or "CHECK blocked_a_b=observed idle_context=observed wake_a_b=observed states_and_queue=verified" not in lines:
        raise RuntimeError("missing blocking or idle checks: " + serial)
    irq_lines = [line for line in lines if line.startswith("IRQ tick=")]
    if len(irq_lines) != 8:
        raise RuntimeError("expected eight actual IRQ observations: " + serial)
    actors = [0,1,2,0,1,0,1,0]
    next_ids = [1,2,0,1,0,1,0,"finish"]
    actions = [1,2,3,4,0,0,0,5]
    states = [515,771,770,513,258,513,258,258]
    ready = [0,0,0,1,1,1,1,1]
    for i, line in enumerate(irq_lines):
        expected = f"IRQ tick={i+1} task={actors[i]} next={next_ids[i]} action={actions[i]} states={states[i]} ready={ready[i]} count="
        if not line.startswith(expected) or not line[len(expected):].isdigit():
            raise RuntimeError("unexpected IRQ ordering: " + serial)
    if hashlib.sha256(guest.read_bytes()).hexdigest() != guest_hash:
        raise RuntimeError("guest image changed while the test was running")
    version = subprocess.run([args.qemu, "--version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    result = {"guest_sha256": guest_hash, "qemu_version": version,
              "serial": serial, "input": "eight guest timer IRQs, two CPU loops and scripted synthetic block/wake events",
              "limitations": "single-CPU QEMU integer-only test; synthetic event script, no generic scheduler, process isolation, FP/SIMD or native Apple hardware"}
    destination = Path(args.output_dir)
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "preemptive-blocking-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print("PASS preemptive block wake idle scenario in QEMU; synthetic evidence saved")


if __name__ == "__main__":
    main()
