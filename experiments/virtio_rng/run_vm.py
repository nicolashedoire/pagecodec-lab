#!/usr/bin/env python3
"""Exercise an isolated VirtIO RNG guest against deterministic fake data."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import selectors
import subprocess
import tempfile
import time


WORKSPACE_LINES = [
    'WORKSPACE_INPUT base.title=base human.title=human assistant.status=assistant',
    'WORKSPACE_CHECK independent=ok human=human assistant=assistant',
    'WORKSPACE_INPUT conflict.base=base conflict.human=human conflict.assistant=assistant',
    'WORKSPACE_CHECK conflict_preserved=ok materialize_rejected=ok',
]


def run(qemu, image, fixture, expected, expected_hex=None, expect_io_merge=False):
    command = [qemu, '-M', 'virt,virtualization=off', '-accel', 'tcg',
               '-cpu', 'cortex-a57', '-smp', '1', '-m', '128M', '-nic', 'none',
               '-nographic', '-monitor', 'none', '-serial', 'stdio', '-no-reboot',
               '-global', 'virtio-mmio.force-legacy=false', '-kernel', str(image)]
    if fixture is not None:
        command += ['-object', f'rng-random,id=rng0,filename={fixture}',
                    '-device', 'virtio-rng-device,rng=rng0']
    started = time.monotonic()
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT)
    captured = bytearray()
    deadline = time.monotonic() + 10
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    terminated_by_runner = False
    try:
        while time.monotonic() < deadline:
            if selector.select(timeout=0.2):
                chunk = os.read(process.stdout.fileno(), 4096)
                if not chunk: break
                captured.extend(chunk)
                complete_lines = captured.split(b'\n')[:-1]
                if any(b'PASS:' in line or b'FAIL:' in line for line in complete_lines): break
            if process.poll() is not None: break
    finally:
        if process.poll() is None:
            terminated_by_runner = True
            process.kill()
        try: process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait()
        selector.close()
        captured.extend(process.stdout.read())
    out = captured.decode('utf-8', errors='replace')
    print(out.strip())
    marker_lines = [line.strip() for line in out.splitlines()
                    if line.startswith(('PASS:', 'FAIL:'))]
    received_lines = [line.strip() for line in out.splitlines() if line.startswith('RECEIVED ')]
    matched = re.fullmatch(r'RECEIVED count=(\d+) hex=([0-9a-f]*)', received_lines[0]) if len(received_lines) == 1 else None
    received = {'count': int(matched.group(1)), 'hex': matched.group(2)} if matched else None
    expected_received = {'count': 32, 'hex': expected_hex} if expected_hex is not None else None
    data_verified = received == expected_received and len(received_lines) == (1 if expected_hex is not None else 0)
    workspace_lines = [line.strip() for line in out.splitlines()
                       if line.startswith(('WORKSPACE_INPUT ', 'WORKSPACE_CHECK '))]
    workspace_verified = workspace_lines == WORKSPACE_LINES
    io_lines = [line.strip() for line in out.splitlines() if line.startswith('IO_MERGE_CHECK ')]
    expected_io_line = ('IO_MERGE_CHECK human=human assistant=device_data data_hex=' + expected_hex
                        if expect_io_merge else None)
    io_merge_verified = io_lines == ([expected_io_line] if expect_io_merge else [])
    return {'console_output': out, 'expected_marker': expected,
            'observed_markers': marker_lines,
            'received': received, 'expected_received': expected_received,
            'workspace_inputs_and_checks': workspace_lines,
            'workspace_verified': workspace_verified,
            'io_merge_checks': io_lines,
            'io_merge_expected': expected_io_line,
            'io_merge_verified': io_merge_verified,
            'verified': marker_lines == [expected] and data_verified and workspace_verified and io_merge_verified,
            'wall_seconds': round(time.monotonic() - started, 6),
            'runner_terminated_process': terminated_by_runner}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--qemu', required=True)
    p.add_argument('--image', type=Path, default=Path('target/aarch64-unknown-none-softfloat/release/virtio-rng-lab'))
    p.add_argument('--report', type=Path, default=Path('run-results/latest.json'),
                   help='write structured run evidence (default: ignored local output)')
    args = p.parse_args()
    if not args.image.is_file(): p.error('built kernel image not found')
    version = subprocess.check_output([args.qemu, '--version'], text=True).splitlines()[0]
    records = []
    with tempfile.TemporaryDirectory(prefix='virtio-rng-fixture-') as temp:
        folder = Path(temp)
        valid = folder/'valid.bin'; valid.write_bytes(bytes(range(256))*16)
        invalid = folder/'invalid.bin'; invalid.write_bytes(bytes([255])*4096)
        print('Valid synthetic source:')
        first = run(args.qemu, args.image, valid, 'PASS: synthetic bytes via VirtIO RNG', bytes(range(32)).hex(), expect_io_merge=True)
        records.append({'scenario': 'valid_synthetic_bytes', 'device_present': True,
                        'input': {'description': 'bytes 0..255 repeated 16 times',
                                  'size_bytes': valid.stat().st_size,
                                  'sha256': hashlib.sha256(valid.read_bytes()).hexdigest(),
                                  'guest_requested_bytes': 32}, **first})
        print('Wrong synthetic source:')
        second = run(args.qemu, args.image, invalid, 'FAIL: unexpected RNG data', bytes([255]*32).hex())
        records.append({'scenario': 'invalid_synthetic_bytes', 'device_present': True,
                        'input': {'description': 'byte 255 repeated 4096 times',
                                  'size_bytes': invalid.stat().st_size,
                                  'sha256': hashlib.sha256(invalid.read_bytes()).hexdigest(),
                                  'guest_requested_bytes': 32}, **second})
        print('Missing device:')
        third = run(args.qemu, args.image, None, 'PASS: no recognized VirtIO entropy device')
        records.append({'scenario': 'no_device', 'device_present': False,
                        'input': None, **third})
    report = {'schema': 'virtio-rng-workspaces-v2', 'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'qemu_version': version, 'host_system': platform.system(), 'host_arch': platform.machine(),
              'guest': {'board': 'virt,virtualization=off', 'cpu': 'cortex-a57', 'accelerator': 'tcg',
                        'vcpus': 1, 'memory_bytes': 128 * 1024 * 1024, 'network': False,
                        'disk': False, 'transport': 'virtio-mmio v2', 'serial': 'PL011'},
              'guest_image_sha256': hashlib.sha256(args.image.read_bytes()).hexdigest(),
              'input_scope': 'Temporary synthetic file read by QEMU; not a physical-device trace.',
              'cases': records, 'all_verified': all(row['verified'] for row in records)}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(f"Structured evidence written; verified={report['all_verified']}")
    if not report['all_verified']: raise SystemExit('One or more scenarios failed')


if __name__ == '__main__': main()
