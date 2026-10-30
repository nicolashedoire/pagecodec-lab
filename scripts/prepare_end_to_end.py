#!/usr/bin/env python3
"""Prepare unchanged source text and frozen, unique offline report plans."""
import argparse
import array
import bisect
import csv
import gzip
import json
import random
import shutil
import struct
import sys
import zipfile
from pathlib import Path

import prepare_operations as operations

ROOT = Path(__file__).resolve().parents[1]
MISSING = -(2**31)
COUNTS = (1, 32, 512, 4096)
METHODS = ('raw', 'bitplane', 'bitplane_adaptive', 'sorted_blocks', 'stream_sweep')


def digest(path):
    return operations.digest(path)


def info(path):
    return {'path': path.relative_to(ROOT).as_posix(), 'bytes': path.stat().st_size, 'sha256': digest(path)}


def ints(path):
    a = array.array('i')
    with path.open('rb') as stream:
        a.fromfile(stream, path.stat().st_size // 4)
    if sys.byteorder != 'little':
        a.byteswap()
    return a


def calibration(path, name, count):
    if name == 'household':
        with path.open(newline='') as f:
            rows = csv.DictReader(f, delimiter=';')
            values = []
            for i, row in enumerate(rows):
                if i == count:
                    break
                v = row['Global_active_power']
                if v not in ('', '?'):
                    # All source measurements have at most three decimal places.
                    whole, _, frac = v.partition('.')
                    if len(frac) > 3:
                        raise ValueError('Unexpected decimal precision')
                    values.append(int(whole) * 1000 + int(frac.ljust(3, '0')))
            return sorted(values)
    with path.open(newline='') as f:
        rows = csv.reader(f)
        index = 0
        if name == 'bike':
            index = next(rows).index('cnt')
        values = []
        for i, row in enumerate(rows):
            if i == count:
                break
            values.append(int(row[index]))
        return sorted(values)


def oracle(xs, ys, queries):
    """Independent Python bin histogram + endpoint snapshots; no Fenwick tree."""
    thresholds = sorted({q[2] for q in queries})
    ranks = {t: i for i, t in enumerate(thresholds)}
    events = []
    for i, (begin, end, threshold) in enumerate(queries):
        events.extend(((begin, i, -1, ranks[threshold]), (end, i, 1, ranks[threshold])))
    events.sort()
    counts, sums = [0]*len(thresholds), [0]*len(thresholds)
    answers = [[0, 0] for _ in queries]
    position = 0
    for endpoint, i, sign, rank in events:
        while position < endpoint:
            x, y = xs[position], ys[position]
            if x != MISSING:
                b = bisect.bisect_right(thresholds, x)-1
                if b >= 0:
                    counts[b] += 1
                    if y != MISSING:
                        sums[b] += y
            position += 1
        answers[i][0] += sign * sum(counts[rank:])
        answers[i][1] += sign * sum(sums[rank:])
    return [{'count': c, 'sum': s} for c, s in answers]


def fingerprint(xs, ys):
    value = 14695981039346656037
    for pair in zip(xs, ys):
        for byte in struct.pack('<ii', *pair):
            value = ((value ^ byte) * 1099511628211) & ((1 << 64)-1)
    return str(value)


def build():
    existing = json.loads((ROOT / 'manifests/operations-v1.json').read_text())
    if any(not (ROOT/c[k]['path']).exists() for c in existing['numeric'] for k in ('x', 'y')):
        derived = operations.build()
        if derived != existing:
            raise ValueError('Operations input manifest changed')
    dest = ROOT / 'data/end-to-end'
    dest.mkdir(parents=True, exist_ok=True)
    sources = {}
    household = dest / 'household.txt'
    with household.open('wb') as out:
        for part in sorted((ROOT / 'data/household').glob('household-text-part-*.txt')):
            with part.open('rb') as f:
                shutil.copyfileobj(f, out)
    household_manifest = json.loads((ROOT / 'manifests/household.json').read_text())
    assert digest(household) == household_manifest['source']['source_sha256']
    sources['household'] = household
    with zipfile.ZipFile(operations.get('covertype.zip')) as z:
        with z.open('covtype.data.gz') as compressed, gzip.GzipFile(fileobj=compressed) as f:
            path = dest / 'covertype.csv'
            with path.open('wb') as out:
                shutil.copyfileobj(f, out)
            sources['covertype'] = path
    with zipfile.ZipFile(operations.get('bike-sharing.zip')) as z:
        with z.open('hour.csv') as f:
            path = dest / 'bike.csv'
            with path.open('wb') as out:
                shutil.copyfileobj(f, out)
            sources['bike'] = path
    cases = []
    for index, case in enumerate(existing['numeric']):
        name, n = case['name'], case['rows']
        for key in ('x', 'y'):
            assert digest(ROOT/case[key]['path']) == case[key]['sha256']
        cal = calibration(sources[name], name, case['calibration_rows'])
        thresholds = sorted({cal[(len(cal)-1)*i//32] for i in range(1, 32)})
        rng = random.Random(20261031+index)
        queries = [(0, n, cal[(len(cal)-1)//2])]
        seen = set(queries)
        lengths = [7*n//8, n//2, min(65536, n//4), min(1024, n//16)]
        for i in range(4095):
            length = max(1, lengths[i % 4])
            while True:
                begin = rng.randrange(n-length+1)
                q = (begin, begin+length, rng.choice(thresholds))
                if q not in seen:
                    break
            queries.append(q)
            seen.add(q)
        xs, ys = ints(ROOT/case['x']['path']), ints(ROOT/case['y']['path'])
        answers = oracle(xs, ys, queries)
        plans = []
        for count in COUNTS:
            qp = dest / f'{name}-{count}.tsv'
            qp.write_text(''.join(f'{a}\t{b}\t{t}\n' for a, b, t in queries[:count]))
            ap = dest / f'{name}-{count}-expected.json'
            ap.write_text(json.dumps(answers[:count], separators=(',', ':')) + '\n')
            plans.append({'query_count': count, 'queries': info(qp), 'expected': info(ap),
                          'row_visits': sum(b-a for a, b, _ in queries[:count]),
                          'distinct_thresholds': len({t for _, _, t in queries[:count]}),
                          'unique_queries': len(set(queries[:count]))})
        cases.append({'name': name, 'format': name, 'source': info(sources[name]),
                      'source_rows': case['source_rows'], 'skip_rows': case['calibration_rows'],
                      'rows': n, 'thresholds': thresholds, 'window_lengths': lengths,
                      'seed': 20261031+index, 'projected_fnv1a64': fingerprint(xs, ys), 'plans': plans})
    return {'schema_version': 1, 'cases': cases, 'protocol': {
        'methods': list(METHODS), 'repetitions': 9, 'query_counts': list(COUNTS),
        'order_seed': 20261031, 'block_rows': 1024,
        'task': 'Offline exact count and masked-sum report; all unique requests supplied before reading source text',
        'primary_time': 'Parent perf_counter around fresh /usr/bin/time child process, launch through output and exit',
        'primary_memory': 'Fresh child process maximum resident set size from /usr/bin/time -l on macOS',
        'included': 'Query-plan reading, complete source text reading, parsing, calibration-row skipping, construction, queries, answer serialization and exit',
        'excluded': 'Download/archive extraction, calibration and query-plan preparation, independent oracle, input hash verification',
        'file_cache': 'OS file cache is not flushed; pre-run hashing warms source files',
        'success_rule': 'At least 10% lower median external elapsed AND at least 10% lower median peak RSS than raw, on two independent sources at 4096 unique queries; disclose all other methods and load levels',
        'scope': 'A standalone reporting application prototype with generated query plans, not a deployed service or interactive trace',
        'no_tuning_after_measurement': True}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record-manifest', action='store_true')
    args = parser.parse_args()
    result = build()
    path = ROOT / 'manifests/end-to-end-v1.json'
    if args.record_manifest:
        path.write_text(json.dumps(result, indent=2)+'\n')
    elif result != json.loads(path.read_text()):
        raise RuntimeError('Prepared files differ from pinned manifest')
    print('Verified raw text sources, 12 unique query-plan prefixes and independent answers.')
