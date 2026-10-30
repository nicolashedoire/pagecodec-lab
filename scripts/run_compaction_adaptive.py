#!/usr/bin/env python3
"""Run matched native compaction policies on new corpora and layouts."""
import argparse
import datetime
import json
import platform
import random
import subprocess
import time
from pathlib import Path

from prepare_compaction import digest

ROOT = Path(__file__).resolve().parents[1]
SOURCE_FILES = ('experiments/compaction_adaptive/compaction_bench.cpp',
                'tests/test_compaction_adaptive.py',
                'scripts/prepare_compaction.py', 'scripts/prepare_compaction_adaptive.py',
                'scripts/run_compaction_adaptive.py', 'manifests/compaction-adaptive-v1.json')


def capture(cmd):
    return subprocess.check_output(cmd, text=True).strip()


def hashes():
    return {p: digest(ROOT / p) for p in SOURCE_FILES}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('measurements/compaction-adaptive'))
    parser.add_argument('--pilot', action='store_true')
    args = parser.parse_args()
    out = ROOT / args.output
    if out.exists() and any(out.iterdir()):
        parser.error('Output directory must be empty')
    manifest = json.loads((ROOT / 'manifests/compaction-adaptive-v1.json').read_text())
    for case in manifest['cases']:
        for field in ('source', 'packed', 'upstream_manifest'):
            item = case[field]
            if digest(ROOT / item['path']) != item['sha256']:
                raise RuntimeError('Frozen input differs: ' + item['path'])
    binary = ROOT / 'build/compaction-adaptive-bench'
    binary.parent.mkdir(exist_ok=True)
    compile_cmd = ['clang++', '-O3', '-std=c++17', '-Wall', '-Wextra', '-Werror',
                   str(ROOT / SOURCE_FILES[0]), '-o', str(binary)]
    subprocess.run(compile_cmd, cwd=ROOT, check=True)
    config = {'recorded_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'pilot': args.pilot, 'sources_sha256': hashes(), 'binary_sha256': digest(binary),
              'compiler': '\n'.join(s for s in capture(['clang++', '--version']).splitlines()
                                    if not s.startswith('InstalledDir:')),
              'platform': platform.system()+' '+platform.release()+' '+platform.machine(),
              'protocol': manifest['protocol']}
    if platform.system() == 'Darwin':
        config.update(machine=capture(['sysctl','hw.memsize','hw.model','machdep.cpu.brand_string']),
                      os=capture(['sw_vers']), vm_before=capture(['vm_stat']),
                      swap_before=capture(['sysctl','vm.swapusage']))
    out.mkdir(parents=True, exist_ok=True)
    (out/'frozen-config.json').write_text(json.dumps(config, indent=2)+'\n')
    rng = random.Random(manifest['protocol']['order_seed'])
    repetitions = 1 if args.pilot else manifest['protocol']['repetitions']
    combinations = [(case, layout, quota) for case in manifest['cases']
                    for layout in manifest['protocol']['layouts']
                    for quota in manifest['protocol']['quotas_by_layout'][layout]]
    samples = []
    with (out/'samples.jsonl').open('w') as log:
        for trial in range(repetitions):
            cases=list(combinations); rng.shuffle(cases)
            for case, layout, quota in cases:
                policies=list(manifest['protocol']['policies'])
                if trial == 0: policies.append('none')
                rng.shuffle(policies)
                pair=[]
                for position, policy in enumerate(policies):
                    seed=manifest['protocol']['layout_seed_base']+trial
                    command=[str(binary),'--input',str(ROOT/case['packed']['path']),
                             '--policy',policy,'--layout',layout,'--seed',str(seed),
                             '--reclaim-fraction',str(quota),'--copies',str(case['copies']),
                             '--segment-bytes',str(manifest['protocol']['segment_bytes']),
                             '--cache-scrub-mib',str(manifest['protocol']['cache_scrub_mib'])]
                    started=time.perf_counter_ns()
                    proc=subprocess.run(command,cwd=ROOT,text=True,capture_output=True,timeout=120)
                    external_ms=(time.perf_counter_ns()-started)/1e6
                    if proc.returncode:
                        raise RuntimeError(f'{case["name"]}/{layout}/{policy}: {proc.stderr}')
                    m=json.loads(proc.stdout)
                    if not m['checksum_ok'] or m['checksum_before']!=m['checksum_after']:
                        raise RuntimeError('Live payload/handle integrity failed')
                    if policy!='none' and not m['target_met']:
                        raise RuntimeError('Policy failed its memory target')
                    if m['mapped_before']-m['mapped_after']!=m['released_bytes']:
                        raise RuntimeError('Memory release accounting failed')
                    row={'case':case['name'],'layout':layout,'reclaim_fraction':quota,
                         'policy':policy,'trial':trial,'order_position':position,'seed':seed,
                         'external_elapsed_ms':external_ms,'measurement':m}
                    samples.append(row);pair.append(row)
                    log.write(json.dumps(row,separators=(',',':'))+'\n');log.flush()
                for field in ('target_bytes','max_reclaimable_bytes','mapped_before','checksum_before'):
                    if len({r['measurement'][field] for r in pair})!=1:
                        raise RuntimeError('Paired policies started from unequal state: '+field)
                competing=[r for r in pair if r['policy']!='none']
                if quota==1:
                    for field in ('released_bytes','bytes_moved','records_moved'):
                        if len({r['measurement'][field] for r in competing})!=1:
                            raise RuntimeError('Full-reclaim invariant failed: '+field)
                print(f'Round {trial+1}/{repetitions}: {case["name"]}/{layout}/{quota:g} verified',flush=True)
    if config['sources_sha256']!=hashes() or digest(binary)!=config['binary_sha256']:
        raise RuntimeError('Source or executable changed during the run')
    report={'schema_version':1,'frozen_config':config,'manifest':manifest,'samples':samples}
    if platform.system()=='Darwin':
        report.update(vm_after=capture(['vm_stat']),swap_after=capture(['sysctl','vm.swapusage']))
    (out/'all-results.json').write_text(json.dumps(report,indent=2)+'\n')
    print(f'Completed {len(samples)} verified fresh processes.',flush=True)


if __name__=='__main__':
    main()
