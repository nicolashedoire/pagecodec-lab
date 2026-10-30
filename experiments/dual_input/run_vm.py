#!/usr/bin/env python3
"""Validate the synthetic input-router trace from a QEMU ARM guest only."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import time

ROOT = Path(__file__).resolve().parent
IMAGE = ROOT / 'target/aarch64-unknown-none-softfloat/release/dual-input-lab'


def expected():
    def frame(step, hx, hy, hf, hr, ax, ay, af, ar, revision, value):
        return {'step': step, 'human': {'x': hx, 'y': hy, 'focus': hf, 'revision': hr},
                'ai': {'x': ax, 'y': ay, 'focus': af, 'revision': ar},
                'shared': {'revision': revision, 'value': value}}
    return [
        frame('initial', 0,0,0,1, 0,0,0,2, 0,100),
        frame('human_click', 25,35,11,3, 0,0,0,2, 0,100),
        frame('ai_click', 25,35,11,3, 45,60,22,4, 0,100),
        frame('cross_scope_denied', 25,35,11,3, 45,60,22,4, 0,100),
        frame('human_commit', 25,35,11,4, 45,60,22,4, 1,101),
        frame('stale_ai_conflict', 25,35,11,4, 45,60,22,4, 1,101),
        frame('focus_denied', 25,35,11,4, 420,20,22,5, 1,101),
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qemu', default='qemu-system-aarch64')
    parser.add_argument('--image', type=Path, default=IMAGE)
    parser.add_argument('--report', type=Path, default=ROOT / 'run-results/latest.json')
    parser.add_argument('--timeout', type=float, default=20.0)
    args = parser.parse_args()
    if not 0 < args.timeout <= 120: parser.error('timeout must be in (0, 120]')
    image = args.image.resolve()
    if not image.is_file(): parser.error('build the guest image first')
    command = [args.qemu, '-M', 'virt,virtualization=off', '-accel', 'tcg', '-cpu',
               'cortex-a57', '-smp', '1', '-m', '128M', '-nic', 'none',
               '-nographic', '-monitor', 'none', '-serial', 'stdio', '-no-reboot',
               '-kernel', str(image)]
    try:
        version = subprocess.check_output([args.qemu, '--version'], text=True).splitlines()[0]
        vm = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT)
    except (OSError, subprocess.CalledProcessError) as exc:
        raise SystemExit(f'QEMU launch failed: {exc}')
    assert vm.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(vm.stdout, selectors.EVENT_READ)
    captured = bytearray()
    deadline = time.monotonic() + args.timeout
    try:
        while time.monotonic() < deadline and len(captured) < 65536:
            if selector.select(timeout=min(.2, max(0.0, deadline-time.monotonic()))):
                chunk = os.read(vm.stdout.fileno(), 4096)
                if not chunk: break
                captured.extend(chunk)
                lines = captured.split(b'\n')[:-1]
                if any(line.startswith((b'PASS:', b'FAIL:')) for line in lines): break
            if vm.poll() is not None: break
    finally:
        if vm.poll() is None: vm.terminate()
        try: vm.wait(timeout=2)
        except subprocess.TimeoutExpired:
            vm.kill(); vm.wait()
        selector.close()
        captured.extend(vm.stdout.read())
        vm.stdout.close()
    output = captured.decode('utf-8', errors='replace')
    lines = output.splitlines()
    markers = [line for line in lines if line.startswith(('PASS:', 'FAIL:'))]
    acknowledgements = [line for line in lines if line.startswith(('AI_ACK ', 'AI_ERR '))]
    try:
        observed = [json.loads(line[6:]) for line in lines if line.startswith('TRACE ')]
    except (ValueError, TypeError) as exc:
        raise SystemExit(f'Invalid guest trace: {exc}')
    verified = (markers == ['PASS: dual-input prototype'] and observed == expected()
                and acknowledgements == ['AI_ACK 17 MOVED 45 60'])
    report = {'schema': 'dual-input-trace-v1',
              'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'qemu_version': version, 'guest_image_sha256': hashlib.sha256(image.read_bytes()).hexdigest(),
              'vm': {'board': 'virt,virtualization=off', 'cpu': 'cortex-a57', 'accelerator': 'tcg',
                     'vcpus': 1, 'memory_bytes': 128 * 1024 * 1024, 'network': False, 'disk': False},
              'trace': observed, 'markers': markers, 'acknowledgements': acknowledgements,
              'verified': verified,
              'notes': ['Synthetic data only.', 'No graphical display, OS access control, AI model or macOS integration.',
                        'Shared-data conflict is detected, not resolved automatically.']}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(output, end='')
    if not verified: raise SystemExit('FAIL: guest trace did not match expected state transitions')
    print('PASS: VM trace independently checked')


if __name__ == '__main__': main()
