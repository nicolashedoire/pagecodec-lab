#!/usr/bin/env python3
"""Run the freestanding kernel only inside a short-lived QEMU ARM VM."""

import argparse
import os
from pathlib import Path
import select
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent
IMAGE = ROOT / 'target/aarch64-unknown-none-softfloat/release/arm-boot'
MARKER = b'ARM Rust kernel: boot OK\r\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qemu', default='qemu-system-aarch64',
                        help='Path to a QEMU system emulator')
    parser.add_argument('--image', type=Path, default=IMAGE,
                        help='Path to the compiled guest ELF image')
    parser.add_argument('--timeout', type=float, default=20.0)
    args = parser.parse_args()
    if not 0 < args.timeout <= 120:
        parser.error('timeout must be greater than zero and at most 120 seconds')
    image = args.image.resolve()
    if not image.is_file():
        parser.error('kernel image missing; run cargo build --release first')
    command = [args.qemu, '-M', 'virt,virtualization=off', '-accel', 'tcg',
               '-cpu', 'cortex-a57', '-smp', '1', '-m', '128M', '-nic', 'none',
               '-nographic', '-monitor', 'none', '-serial', 'stdio',
               '-no-reboot', '-kernel', str(image)]
    deadline = time.monotonic() + args.timeout
    data = bytearray()
    try:
        vm = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError as error:
        print(f'Unable to launch the QEMU VM: {error}', file=sys.stderr)
        return 1
    assert vm.stdout is not None
    try:
        while time.monotonic() < deadline and len(data) < 65536:
            ready, _, _ = select.select([vm.stdout], [], [],
                                        max(0.0, min(0.25, deadline-time.monotonic())))
            if ready:
                chunk = os.read(vm.stdout.fileno(), 4096)
                if not chunk:
                    break
                data.extend(chunk)
                if MARKER in data:
                    print(data.decode('utf-8', errors='replace'), end='')
                    print('PASS: expected serial output from the QEMU VM')
                    return 0
            elif vm.poll() is not None:
                break
        print(data.decode('utf-8', errors='replace'), end='', file=sys.stderr)
        print('FAIL: QEMU did not emit the expected boot marker', file=sys.stderr)
        return 1
    finally:
        if vm.poll() is None:
            vm.terminate()
            try:
                vm.wait(timeout=5)
            except subprocess.TimeoutExpired:
                vm.kill()
                vm.wait()
        vm.stdout.close()


if __name__ == '__main__':
    raise SystemExit(main())
