#!/usr/bin/env python3
"""Run paired, shuffled, fresh-process native compaction measurements."""
import argparse
import datetime
import hashlib
import json
import platform
import random
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda: f.read(2**20), b''):
            h.update(b)
    return h.hexdigest()


def capture(command):
    return subprocess.check_output(command, text=True).strip()


def source_hashes():
    names = ['Makefile', 'experiments/compaction/compaction_bench.cpp',
             'tests/test_compaction.py', 'scripts/prepare_compaction.py',
             'scripts/run_compaction.py', 'manifests/compaction-v1.json']
    return {name: digest(ROOT / name) for name in names}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('measurements/compaction'))
    parser.add_argument('--pilot', action='store_true', help='One round; distinct pilot label, not final evidence')
    args = parser.parse_args()
    out = ROOT / args.output
    if out.exists() and any(out.iterdir()):
        parser.error('Output directory must be empty')
    manifest = json.loads((ROOT / 'manifests/compaction-v1.json').read_text())
    for case in manifest['cases']:
        for name in ('source', 'packed'):
            item = case[name]
            if digest(ROOT / item['path']) != item['sha256']:
                raise RuntimeError('Input changed: ' + item['path'])
    binary = ROOT / 'build/compaction-bench'
    subprocess.run(['make', 'compaction', 'CXX=clang++'], cwd=ROOT, check=True)
    frozen = {'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'sources_sha256': source_hashes(), 'binary_sha256': digest(binary),
              'platform': platform.system() + ' ' + platform.release() + ' ' + platform.machine(),
              'compiler': '\n'.join(line for line in capture(['clang++', '--version']).splitlines()
                                    if not line.startswith('InstalledDir:')),
              'pilot': args.pilot, 'protocol': manifest['protocol']}
    if platform.system() == 'Darwin':
        frozen.update(machine=capture(['sysctl', 'hw.memsize', 'hw.model', 'machdep.cpu.brand_string']),
                      os=capture(['sw_vers']), vm_before=capture(['vm_stat']),
                      swap_before=capture(['sysctl', 'vm.swapusage']))
    out.mkdir(parents=True, exist_ok=True)
    (out / 'frozen-config.json').write_text(json.dumps(frozen, indent=2) + '\n')
    rng = random.Random(manifest['protocol']['order_seed'])
    repetitions = 1 if args.pilot else manifest['protocol']['repetitions']
    combinations = [(case, layout, fraction) for case in manifest['cases']
                    for layout in manifest['protocol']['layouts']
                    for fraction in manifest['protocol']['quotas_by_layout'][layout]]
    samples = []
    with (out / 'samples.jsonl').open('w') as stream:
        for trial in range(repetitions):
            shuffled = list(combinations)
            rng.shuffle(shuffled)
            for case, layout, fraction in shuffled:
                policies = list(manifest['protocol']['policies'])
                if trial == 0:
                    policies.append('none')
                rng.shuffle(policies)
                pair = []
                for position, policy in enumerate(policies):
                    seed = manifest['protocol']['layout_seed_base'] + trial
                    command = [str(binary), '--input', str(ROOT / case['packed']['path']),
                               '--policy', policy, '--layout', layout, '--seed', str(seed),
                               '--reclaim-fraction', str(fraction),
                               '--segment-bytes', str(manifest['protocol']['segment_bytes']),
                               '--cache-scrub-mib', str(manifest['protocol']['cache_scrub_mib']),
                               '--copies', str(case['copies'])]
                    start = time.perf_counter_ns()
                    run = subprocess.run(command, text=True, capture_output=True, timeout=120)
                    external_ms = (time.perf_counter_ns() - start) / 1e6
                    if run.returncode:
                        raise RuntimeError(f'{case["name"]}/{layout}/{policy}: {run.stderr}')
                    result = json.loads(run.stdout)
                    if not result['checksum_ok']:
                        raise RuntimeError('Live payload corruption')
                    if policy != 'none' and not result['target_met']:
                        raise RuntimeError('Compaction did not meet reclaim target')
                    if result['released_bytes'] != result['mapped_before'] - result['mapped_after']:
                        raise RuntimeError('Release accounting mismatch')
                    sample = {'case': case['name'], 'layout': layout, 'reclaim_fraction': fraction,
                              'policy': policy,
                              'trial': trial, 'order_position': position, 'seed': seed,
                              'external_elapsed_ms': external_ms, 'measurement': result}
                    samples.append(sample)
                    pair.append(sample)
                    stream.write(json.dumps(sample, separators=(',', ':')) + '\n')
                    stream.flush()
                targets = {s['measurement']['target_bytes'] for s in pair}
                before = {s['measurement']['mapped_before'] for s in pair}
                if len(targets) != 1 or len(before) != 1:
                    raise RuntimeError('Paired methods received different arenas or quotas')
                print(f'Round {trial+1}/{repetitions}: {case["name"]}/{layout}/{fraction:g} verified', flush=True)
    if frozen['sources_sha256'] != source_hashes() or digest(binary) != frozen['binary_sha256']:
        raise RuntimeError('Frozen code changed during measurement')
    result = {'schema_version': 1, 'frozen_config': frozen, 'manifest': manifest, 'samples': samples}
    if platform.system() == 'Darwin':
        result.update(vm_after=capture(['vm_stat']), swap_after=capture(['sysctl', 'vm.swapusage']))
    (out / 'all-results.json').write_text(json.dumps(result, indent=2) + '\n')
    print(f'Completed {len(samples)} verified fresh processes.', flush=True)


if __name__ == '__main__':
    main()
