#!/usr/bin/env python3
"""Run the page-codec comparison; verify input hashes and freeze metadata first."""
import argparse
import csv
import datetime
import hashlib
import json
import platform
import subprocess
from pathlib import Path


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def capture(command):
    return subprocess.check_output(command, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, default=Path('manifests/household.json'))
    parser.add_argument('--output', type=Path, default=Path('measurements/household'))
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('Apple libcompression comparison requires macOS. Use make test on other systems.')
    root = Path(__file__).resolve().parents[1]
    manifest_path = (root / args.manifest).resolve()
    manifest = json.loads(manifest_path.read_text())
    out = (root / args.output).resolve()
    if out.exists() and any(out.iterdir()):
        parser.error('Output directory is not empty; choose a new directory to preserve existing measurements.')
    out.mkdir(parents=True, exist_ok=True)
    for item in manifest['datasets']:
        path = root / item['path']
        if path.stat().st_size != item['size'] or digest(path) != item['sha256']:
            raise RuntimeError(f"Input differs from manifest: {item['name']}")
    sources = {name: digest(root / name) for name in ('codec.c', 'codec.h', 'compare.c')}
    binary = root / 'build/compare-measured'
    binary.parent.mkdir(exist_ok=True)
    flags = ['-O3', '-Wall', '-Wextra', '-Werror']
    subprocess.run(['clang', *flags, str(root / 'compare.c'), str(root / 'codec.c'),
                    '-lcompression', '-o', str(binary)], check=True)
    compiler = '\n'.join(line for line in capture(['clang', '--version']).splitlines()
                         if not line.startswith('InstalledDir:'))
    frozen = {
        'frozen_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'sources_sha256': sources, 'binary_sha256': digest(binary),
        'manifest_sha256': digest(manifest_path), 'compiler': compiler, 'compiler_flags': flags,
        'machine': capture(['sysctl', 'hw.memsize', 'hw.model', 'machdep.cpu.brand_string']),
        'os': capture(['sw_vers']), 'protocol': manifest['protocol'],
        'page_bytes': 16384, 'repetitions': 9, 'first_seed': 20261030,
        'before_vm_stat': capture(['vm_stat']), 'before_swap': capture(['sysctl', 'vm.swapusage']),
    }
    (out / 'frozen-config.json').write_text(json.dumps(frozen, indent=2) + '\n')
    records, flat = [], []
    for i, item in enumerate(manifest['datasets']):
        print(f"[{i+1}/{len(manifest['datasets'])}] {item['name']} ({item['size']} bytes)", flush=True)
        command = [str(binary), '--input', str(root / item['path']), '--page-kib', '16',
                   '--repetitions', '9', '--seed', str(20261030 + i)]
        result = json.loads(subprocess.run(command, check=True, capture_output=True,
                                           text=True, timeout=300).stdout)
        if len(result['results']) != 5 or not all(r['verified'] for r in result['results']):
            raise RuntimeError('Comparator did not verify all five methods')
        result['input_path'] = item['path']
        result['dataset'] = item
        (out / (item['name'] + '.json')).write_text(json.dumps(result, indent=2) + '\n')
        records.append(result)
        for row in result['results']:
            flat.append({'dataset': item['name'], 'category': item['category'], 'input_bytes': item['size'],
                         'method': row['method'], 'total_storage_bytes': row['total_storage_bytes'],
                         'ratio': row['total_storage_bytes'] / item['size'],
                         'encode_ms': row['encode_ms'], 'decode_ms': row['decode_ms'],
                         'encode_min_ms': min(row['encode_samples_ms']),
                         'encode_max_ms': max(row['encode_samples_ms']),
                         'decode_min_ms': min(row['decode_samples_ms']),
                         'decode_max_ms': max(row['decode_samples_ms']), 'verified': row['verified']})
    for name, expected in sources.items():
        if digest(root / name) != expected:
            raise RuntimeError(f'Source changed during measurements: {name}')
    summary = {'frozen_config': frozen, 'corpus': records, 'after_vm_stat': capture(['vm_stat']),
               'after_swap': capture(['sysctl', 'vm.swapusage'])}
    (out / 'all-results.json').write_text(json.dumps(summary, indent=2) + '\n')
    with (out / 'all-results.csv').open('w', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(flat[0]))
        writer.writeheader()
        writer.writerows(flat)
    print(f'Completed {len(records)} inputs and {len(flat)} method cases; all round trips exact.')


if __name__ == '__main__':
    main()
