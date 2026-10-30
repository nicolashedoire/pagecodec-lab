#!/usr/bin/env python3
"""Exercise restart persistence with exclusively created disposable VM images."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time

SIZE = 1024 * 1024
BLOCK = 512
OFFSET = 4 * BLOCK


def digest(data):
    return hashlib.sha256(data).hexdigest()


def fnv(data):
    h = 2166136261
    for byte in data:
        h = ((h ^ byte) * 16777619) & 0xffffffff
    return h


def create(path, data):
    # The runner does not accept image paths. A new private directory and O_EXCL
    # guarantee every disk used here is a disposable, newly created fixture.
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, 'wb') as f:
        f.write(data)


def boot(qemu, guest, image, marker, legacy):
    blocks = [
        {"driver": "file", "filename": str(image), "node-name": "file0"},
        {"driver": "raw", "file": "file0", "node-name": "disk0"},
    ]
    command = [qemu, '-M', 'virt,virtualization=off', '-accel', 'tcg', '-cpu', 'cortex-a57',
               '-smp', '1', '-m', '128M', '-nodefaults', '-display', 'none', '-nic', 'none',
               '-monitor', 'none', '-serial', 'stdio', '-no-reboot', '-kernel', str(guest),
               '-global', 'virtio-mmio.force-legacy=' + ('true' if legacy else 'false'),
               '-blockdev', json.dumps(blocks[0]), '-blockdev', json.dumps(blocks[1]),
               '-device', 'virtio-blk-device,drive=disk0']
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    received = bytearray()
    try:
        with selectors.DefaultSelector() as sel:
            sel.register(process.stdout, selectors.EVENT_READ)
            end = time.monotonic() + 30
            while time.monotonic() < end:
                if sel.select(0.2):
                    received.extend(os.read(process.stdout.fileno(), 4096))
                    if any(line.startswith((b'PASS ', b'FAIL ', b'REJECT ')) for line in received.split(b'\n')[:-1]):
                        break
                if process.poll() is not None:
                    raise RuntimeError('QEMU exited: ' + received.decode(errors='replace'))
            else:
                raise TimeoutError('guest timeout: ' + received.decode(errors='replace'))
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    serial = received.decode('ascii', errors='replace').replace('\r', '')
    if marker not in serial.splitlines():
        raise RuntimeError('unexpected guest result: ' + serial)
    return serial


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--qemu', required=True)
    p.add_argument('--guest', required=True)
    p.add_argument('--output-dir', required=True)
    p.add_argument('--legacy', action='store_true', help='test legacy instead of modern VirtIO-MMIO')
    args = p.parse_args()
    guest = Path(args.guest)
    guest_hash = digest(guest.read_bytes())
    version = subprocess.run([args.qemu, '--version'], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    with tempfile.TemporaryDirectory(prefix='workspace-persistence-') as directory:
        base = bytearray(SIZE)
        base[:8] = b'VMINIT01'
        image = Path(directory) / 'fixture.raw'
        create(image, base)
        first = boot(args.qemu, guest, image, 'PASS save synthetic bridge record', args.legacy)
        saved = image.read_bytes()
        sector = saved[OFFSET:OFFSET+BLOCK]
        if saved[:OFFSET] != base[:OFFSET] or saved[OFFSET+BLOCK:] != base[OFFSET+BLOCK:]:
            raise RuntimeError('write escaped designated fixture sector')
        if sector[:8] != b'WSPERS01' or sector[8:10] != bytes([13,15]) or sector[10:38] != b'human-versionassistant-ready' or int.from_bytes(sector[508:], 'little') != fnv(sector[:508]):
            raise RuntimeError('invalid saved record bytes')
        restore = bytearray(saved)
        restore[:8] = b'VMLOAD01'
        # Only the runner-owned control sector is changed between boots.
        with image.open('r+b') as f:
            f.write(restore[:BLOCK])
        second = boot(args.qemu, guest, image, 'PASS restore synthetic bridge record', args.legacy)
        if image.read_bytes() != restore:
            raise RuntimeError('restore altered fixture')
        corrupt = bytearray(restore)
        corrupt[OFFSET+10] ^= 1
        corrupt_image = Path(directory) / 'checksum-corrupt.raw'
        create(corrupt_image, corrupt)
        rejected_checksum = boot(args.qemu, guest, corrupt_image, 'REJECT malformed record', args.legacy)
        if corrupt_image.read_bytes() != corrupt:
            raise RuntimeError('rejection altered corrupt fixture')
        malformed = bytearray(restore)
        malformed[OFFSET+8] = 255
        malformed[OFFSET+508:OFFSET+512] = fnv(malformed[OFFSET:OFFSET+508]).to_bytes(4, 'little')
        malformed_image = Path(directory) / 'length-corrupt.raw'
        create(malformed_image, malformed)
        rejected_length = boot(args.qemu, guest, malformed_image, 'REJECT malformed record', args.legacy)
        if malformed_image.read_bytes() != malformed:
            raise RuntimeError('rejection altered malformed fixture')
    if digest(guest.read_bytes()) != guest_hash:
        raise RuntimeError('guest image changed during run')
    result = {'guest_sha256': guest_hash, 'qemu_version': version,
              'fixture': 'fresh disposable 1 MiB image; control sector 0 and data sector 4',
              'virtio_mmio_mode': 'legacy' if args.legacy else 'modern',
              'save_serial': first, 'saved_record_sha256': digest(sector), 'restore_serial': second,
              'corrupt_checksum_serial': rejected_checksum, 'invalid_length_with_valid_checksum_serial': rejected_length,
              'checks': 'unmodified neighboring sectors; restore and rejected images unchanged',
              'limitations': 'FNV is not authentication; no transactions or crash durability; guest-owned synthetic fixtures only'}
    out = Path(args.output_dir)
    out.mkdir(parents=True, exist_ok=True)
    (out / 'workspace-persistence-evidence.json').write_text(json.dumps(result, indent=2) + '\n')
    print('PASS save, restart/restore, corruption and malformed-length rejection in QEMU')


if __name__ == '__main__':
    main()
