#!/usr/bin/env python3
"""Freeze sources and inputs, then run all three experiments sequentially."""
import argparse
import datetime
import hashlib
import json
import platform
import re
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
METHODS = {'query': ['raw', 'for', 'bitplane_eager', 'bitplane_progressive', 'bitplane_adaptive', 'rle'],
           'recipe': ['materialized', 'compressed_blocks', 'recipe', 'recipe_cache', 'recipe_direct'],
           'strings': ['concat', 'intern', 'bounded_overlap', 'zlib_blocks']}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(args):
    return subprocess.check_output(args, text=True).strip()


def source_hashes():
    paths = [ROOT / p for p in ('codec.c', 'codec.h', 'Makefile', 'scripts/prepare_operations.py',
                               'scripts/run_operations.py')]
    paths.extend(sorted((ROOT / 'experiments').rglob('*.cpp')))
    paths.extend(sorted((ROOT / 'experiments').rglob('*.h')))
    paths.extend(sorted((ROOT / 'experiments').rglob('*.hpp')))
    return {p.relative_to(ROOT).as_posix(): digest(p) for p in paths}


def command(family, case, repetitions, output, method=None):
    binary = {'query': 'query-bench', 'recipe': 'recipe-bench', 'strings': 'string-bench'}[family]
    args = [str(ROOT / 'build' / binary)]
    if family != 'strings':
        args += ['--x', str(ROOT / case['x']['path']), '--y', str(ROOT / case['y']['path']),
                 '--repetitions', str(repetitions)]
    if family == 'query':
        args += ['--thresholds', ','.join(map(str, case['thresholds']))]
    elif family == 'recipe':
        args += ['--tile-rows', '256', '--cache-tiles', '64']
    else:
        args += ['--input', str(ROOT / case['path']), '--trials', str(repetitions),
                 '--queries', '20000', '--block-bytes', '4096', '--candidate-cap', '64',
                 '--max-overlap-bytes', '256']
    args += ['--seed', '20261030', '--output', str(output)]
    if method:
        args += ['--only-method', method]
    return args


def redact(value):
    if isinstance(value, dict):
        return {k: redact(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    if isinstance(value, str):
        return value.replace(str(ROOT) + '/', '')
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('measurements/operations'))
    parser.add_argument('--skip-isolated', action='store_true', help='Omit separate one-method peak-RSS probes')
    args = parser.parse_args()
    out = (ROOT / args.output).resolve()
    if out.exists() and any(out.iterdir()):
        parser.error('Output is not empty; choose a new directory')
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = ROOT / 'manifests/operations-v1.json'
    manifest = json.loads(manifest_path.read_text())
    for case in manifest['numeric']:
        for key in ('x', 'y'):
            item = case[key]
            if digest(ROOT / item['path']) != item['sha256']:
                raise RuntimeError('Input hash mismatch: ' + item['path'])
    for item in manifest['strings']:
        if digest(ROOT / item['path']) != item['sha256']:
            raise RuntimeError('Input hash mismatch: ' + item['path'])
    subprocess.run(['make', 'operations', 'CC=clang', 'CXX=clang++'], cwd=ROOT, check=True)
    sources = source_hashes()
    frozen = {'frozen_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'sources_sha256': sources, 'manifest_sha256': digest(manifest_path),
              'compiler': '\n'.join(x for x in capture(['clang++', '--version']).splitlines()
                                    if not x.startswith('InstalledDir:')),
              'platform': platform.system() + ' ' + platform.release() + ' ' + platform.machine(),
              'protocol': manifest['protocol'],
              'binary_sha256': {p: digest(ROOT / 'build' / p)
                                for p in ('query-bench', 'recipe-bench', 'string-bench')}}
    if platform.system() == 'Darwin':
        frozen.update(machine=capture(['sysctl', 'hw.memsize', 'hw.model', 'machdep.cpu.brand_string']),
                      os=capture(['sw_vers']), before_vm_stat=capture(['vm_stat']),
                      before_swap=capture(['sysctl', 'vm.swapusage']))
    (out / 'frozen-config.json').write_text(json.dumps(frozen, indent=2) + '\n')
    cases = [('query', c) for c in manifest['numeric']]
    cases += [('recipe', c) for c in manifest['numeric']]
    cases += [('strings', c) for c in manifest['strings']]
    records = []
    for i, (family, case) in enumerate(cases):
        label = family + '-' + case['name']
        print(f'[{i+1}/{len(cases)}] {label}', flush=True)
        target = out / (label + '.json')
        start = time.monotonic()
        subprocess.run(command(family, case, 9, target), cwd=ROOT, check=True, timeout=600)
        result = redact(json.loads(target.read_text()))
        result['case'] = case
        result['family'] = family
        result['runner_elapsed_seconds'] = time.monotonic()-start
        result['isolated_peak_rss'] = []
        if not args.skip_isolated and platform.system() == 'Darwin':
            probes = out / 'isolated'
            probes.mkdir(exist_ok=True)
            for method in METHODS[family]:
                probe = probes / (label + '-' + method + '.json')
                log = probes / (label + '-' + method + '.time.txt')
                subprocess.run(['/usr/bin/time', '-l', '-o', str(log), *command(family, case, 1, probe, method)],
                               cwd=ROOT, check=True, timeout=300)
                stats = log.read_text()
                match = re.search(r'(\d+)\s+maximum resident set size', stats)
                if not match:
                    raise RuntimeError('Peak RSS missing from time output')
                result['isolated_peak_rss'].append({'method': method, 'bytes': int(match.group(1)),
                    'scope': 'Separate process high-water RSS including construction, input loading, validation and workload; not steady-state store size'})
                probe.write_text(json.dumps(redact(json.loads(probe.read_text())), indent=2) + '\n')
        target.write_text(json.dumps(result, indent=2) + '\n')
        records.append(result)
    if source_hashes() != sources:
        raise RuntimeError('Sources changed during measurements')
    report = {'frozen_config': frozen, 'experiments': records,
              'measurement_note': 'One desktop session; isolated RSS probes are separate one-trial runs and are not timing evidence.'}
    if platform.system() == 'Darwin':
        report.update(after_vm_stat=capture(['vm_stat']), after_swap=capture(['sysctl', 'vm.swapusage']))
    (out / 'all-results.json').write_text(json.dumps(report, indent=2) + '\n')
    print('Completed all eight real-data experiment cases.', flush=True)


if __name__ == '__main__':
    main()
