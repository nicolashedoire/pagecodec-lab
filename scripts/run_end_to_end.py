#!/usr/bin/env python3
"""Measure complete offline-report processes, with paired trials and peak RSS."""
import argparse
import datetime
import hashlib
import json
import platform
import random
import re
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda: f.read(2**20), b''):
            h.update(b)
    return h.hexdigest()


def sources():
    files = ['Makefile', 'experiments/end_to_end/report_bench.cpp',
             'tests/test_end_to_end.py', 'scripts/prepare_end_to_end.py',
             'scripts/run_end_to_end.py', 'manifests/end-to-end-v1.json']
    return {p: digest(ROOT/p) for p in files}


def capture(args):
    return subprocess.check_output(args, text=True).strip()


def redact(value):
    if isinstance(value, dict):
        return {k: redact(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v) for v in value]
    return value.replace(str(ROOT)+'/', '') if isinstance(value, str) else value


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, default=Path('measurements/end-to-end'))
    args = p.parse_args()
    out = (ROOT/args.output).resolve()
    if out.exists() and any(out.iterdir()):
        p.error('Output directory must be empty')
    if platform.system() not in ('Darwin', 'Linux'):
        p.error('This runner requires macOS or Linux /usr/bin/time')
    manifest = json.loads((ROOT/'manifests/end-to-end-v1.json').read_text())
    for case in manifest['cases']:
        items = [case['source']]
        for plan in case['plans']:
            items.extend((plan['queries'], plan['expected']))
        for item in items:
            if digest(ROOT/item['path']) != item['sha256']:
                raise RuntimeError('Input hash mismatch: '+item['path'])
    subprocess.run(['make', 'end-to-end', 'CXX=clang++'], cwd=ROOT, check=True)
    binary = ROOT/'build/report-bench'
    frozen_sources = sources()
    frozen = {'frozen_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'sources_sha256': frozen_sources, 'binary_sha256': digest(binary),
              'protocol': manifest['protocol'],
              'compiler': '\n'.join(s for s in capture(['clang++', '--version']).splitlines()
                                    if not s.startswith('InstalledDir:')),
              'platform': platform.system()+' '+platform.release()+' '+platform.machine(),
              'parent_clock': time.get_clock_info('perf_counter').implementation}
    if platform.system() == 'Darwin':
        frozen.update(machine=capture(['sysctl','hw.memsize','hw.model','machdep.cpu.brand_string']),
                      os=capture(['sw_vers']), before_vm_stat=capture(['vm_stat']),
                      before_swap=capture(['sysctl','vm.swapusage']))
    out.mkdir(parents=True, exist_ok=True)
    for sub in ('reports', 'time-logs', 'plans'):
        (out/sub).mkdir()
    (out/'frozen-config.json').write_text(json.dumps(frozen, indent=2)+'\n')
    records = []
    rng = random.Random(manifest['protocol']['order_seed'])
    repetitions = manifest['protocol']['repetitions']
    for case in manifest['cases']:
        for plan in case['plans']:
            label = case['name']+'-'+str(plan['query_count'])
            expected = json.loads((ROOT/plan['expected']['path']).read_text())
            (out/'plans'/f'{label}.tsv').write_bytes((ROOT/plan['queries']['path']).read_bytes())
            rows = []
            fingerprints = set()
            print(f'{label}: {repetitions} fresh-process rounds × 5 methods', flush=True)
            with tempfile.TemporaryDirectory(prefix='report-run-', dir=ROOT/'build') as tmp:
                temporary = Path(tmp)/'report.json'
                for trial in range(repetitions):
                    order = list(manifest['protocol']['methods'])
                    rng.shuffle(order)
                    for position, method in enumerate(order):
                        log = out/'time-logs'/f'{label}-{trial}-{method}.txt'
                        command = [str(binary),'--input',str(ROOT/case['source']['path']),
                                   '--format',case['format'],'--rows',str(case['rows']),
                                   '--skip-rows',str(case['skip_rows']),'--method',method,
                                   '--queries',str(ROOT/plan['queries']['path']),'--output',str(temporary)]
                        timing = ['/usr/bin/time','-l' if platform.system()=='Darwin' else '-v','-o',str(log)]
                        started = time.perf_counter_ns()
                        completed = subprocess.run(timing+command, cwd=ROOT, capture_output=True,
                                                   text=True, timeout=180)
                        elapsed = (time.perf_counter_ns()-started)/1e6
                        if completed.returncode:
                            raise RuntimeError(f'{label}/{method} failed: '+completed.stderr)
                        result = redact(json.loads(temporary.read_text()))
                        answers = [{'count':a['count'],'sum':a['sum']} for a in result.pop('answers')]
                        if answers != expected:
                            raise RuntimeError(f'Independent oracle disagreement: {label}/{method}')
                        if result['evaluation_rows'] != case['rows'] or result['query_count'] != plan['query_count']:
                            raise RuntimeError('Application row/query count mismatch')
                        fingerprints.add(result['projected_fnv1a64'])
                        if result['projected_fnv1a64'] != case['projected_fnv1a64']:
                            raise RuntimeError('Parsed projection fingerprint differs from independent Python conversion')
                        canonical = json.dumps(answers, separators=(',', ':'))+'\n'
                        if trial == 0 and position == 0:
                            (out/'reports'/f'{label}.answers.json').write_text(canonical)
                        text = log.read_text()
                        if platform.system() == 'Darwin':
                            match = re.search(r'(\d+)\s+maximum resident set size',text)
                            peak = int(match.group(1)) if match else None
                        else:
                            match = re.search(r'Maximum resident set size \(kbytes\):\s*(\d+)',text)
                            peak = int(match.group(1))*1024 if match else None
                            log.write_text(text.replace(str(ROOT)+'/', '').replace(str(temporary), 'temporary-report.json'))
                        if not peak:
                            raise RuntimeError('Missing external peak RSS')
                        rows.append({'method': method, 'trial': trial, 'order_position': position,
                                     'external_elapsed_ms': elapsed, 'peak_rss_bytes': peak,
                                     'answers_sha256': hashlib.sha256(canonical.encode()).hexdigest(),
                                     'independent_answers_verified': True, 'application': result})
                    print(f'  completed paired round {trial+1}/{repetitions}', flush=True)
            if len(fingerprints) != 1:
                raise RuntimeError('Methods parsed different projected values')
            record = {'case': case['name'], 'source': case['source'], 'rows':case['rows'],
                      'plan': plan, 'samples': rows}
            records.append(record)
            (out/f'{label}.json').write_text(json.dumps(record, indent=2)+'\n')
    if frozen_sources != sources() or digest(binary) != frozen['binary_sha256']:
        raise RuntimeError('Frozen source or executable changed during measurements')
    result = {'schema_version': 1, 'frozen_config': frozen, 'cases': records}
    if platform.system() == 'Darwin':
        result.update(after_vm_stat=capture(['vm_stat']),after_swap=capture(['sysctl','vm.swapusage']))
    (out/'all-results.json').write_text(json.dumps(result, indent=2)+'\n')
    print(f'Completed {len(records)*repetitions*5} verified whole-process measurements.', flush=True)


if __name__ == '__main__':
    main()
