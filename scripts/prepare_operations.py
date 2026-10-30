#!/usr/bin/env python3
"""Prepare pinned, real inputs for the three operation-level experiments."""
import argparse
import array
import csv
import gzip
import hashlib
import io
import json
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MISSING = -(2**31)
CMU_COMMIT = '74790861f652b15e4ac49015a90074ad62a27690'
SOURCES = {
    'covertype.zip': ('https://archive.ics.uci.edu/static/public/31/covertype.zip',
                     '89a975c2457cd48e824238ae43c5a3cb762e42c4b4078d9b44a4514055105f6d'),
    'bike-sharing.zip': ('https://archive.ics.uci.edu/static/public/275/bike%2Bsharing%2Bdataset.zip',
                        'b70182d0d0508e9abbb79306ce5c0cec34869000f8220175ac83d11dbe845401'),
    'cmudict.dict': (f'https://raw.githubusercontent.com/cmusphinx/cmudict/{CMU_COMMIT}/cmudict.dict',
                     '81917843c7f44ce2b094ac63873c2c7a4cf802040792c455ba3ca406891c3d22'),
    'cmudict.LICENSE': (f'https://raw.githubusercontent.com/cmusphinx/cmudict/{CMU_COMMIT}/LICENSE',
                       'bd4ce8e44170a5f9f481310ca85c51de3c4f851a65e679b40e603b143bd3542a'),
}


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(2**20), b''):
            h.update(chunk)
    return h.hexdigest()


def get(name):
    path = ROOT / 'data/downloads' / name
    path.parent.mkdir(parents=True, exist_ok=True)
    url, expected = SOURCES[name]
    if not path.exists():
        subprocess.run(['curl', '-fsSL', '--retry', '2', '--max-time', '180', url, '-o', str(path)], check=True)
    if digest(path) != expected:
        raise RuntimeError('Pinned source hash mismatch: ' + name)
    return path


def file_info(path):
    return {'path': path.relative_to(ROOT).as_posix(), 'bytes': path.stat().st_size, 'sha256': digest(path)}


def numeric(name, x, y, expected_rows, fields, citation):
    if len(x) != expected_rows or len(y) != expected_rows:
        raise RuntimeError('Unexpected row count for ' + name)
    split = expected_rows // 5
    calibration = sorted(v for v in x[:split] if v != MISSING)
    thresholds = [calibration[int((len(calibration)-1)*q)] for q in (.1, .5, .9)]
    paths = []
    for label, values in [('x', x), ('y', y)]:
        path = ROOT / f'data/operations/{name}-{label}.i32'
        output = array.array('i', values[split:])
        if output.itemsize != 4:
            raise RuntimeError('A 32-bit array int is required')
        if sys.byteorder != 'little':
            output.byteswap()
        with path.open('wb') as stream:
            output.tofile(stream)
        paths.append(file_info(path))
    return {'name': name, 'source_rows': expected_rows, 'calibration_rows': split,
            'rows': expected_rows-split, 'fields': fields, 'thresholds': thresholds,
            'threshold_rule': '10th/50th/90th order-statistic quantiles of valid x in first 20%; floor index',
            'evaluation_split': 'Remaining 80% in source order, no repetition or shuffling',
            'citation': citation, 'x': paths[0], 'y': paths[1]}


def build():
    (ROOT / 'data/operations').mkdir(parents=True, exist_ok=True)
    household_manifest = json.loads((ROOT / 'manifests/household.json').read_text())
    item = next(d for d in household_manifest['datasets'] if 'column' in d['name'])
    path = ROOT / item['path']
    if not path.exists():
        subprocess.run([sys.executable, str(ROOT / 'scripts/prepare_household.py')], check=True)
    if digest(path) != item['sha256']:
        raise RuntimeError('Household numeric source mismatch')
    n = household_manifest['conversion']['rows']
    columns = []
    with path.open('rb') as stream:
        for index in (1, 2):
            stream.seek(index*n*4)
            a = array.array('i')
            a.fromfile(stream, n)
            if sys.byteorder != 'little':
                a.byteswap()
            columns.append(a)
    cases = [numeric('household', *columns, n,
                     ['Global_active_power x1000', 'Global_reactive_power x1000'],
                     'Hebrail & Berard (2006), UCI, https://doi.org/10.24432/C58K54, CC-BY-4.0')]
    x, y = array.array('i'), array.array('i')
    with zipfile.ZipFile(get('covertype.zip')) as archive:
        with archive.open('covtype.data.gz') as member, gzip.GzipFile(fileobj=member) as stream:
            for line in stream:
                row = line.split(b',')
                if len(row) != 55:
                    raise RuntimeError('Unexpected Covertype fields')
                x.append(int(row[0])); y.append(int(row[5]))
    cases.append(numeric('covertype', x, y, 581012,
                         ['Elevation (m)', 'Horizontal_Distance_To_Roadways (m)'],
                         'Blackard (1998), UCI, https://doi.org/10.24432/C50K5N, CC-BY-4.0'))
    x, y = array.array('i'), array.array('i')
    with zipfile.ZipFile(get('bike-sharing.zip')) as archive:
        with archive.open('hour.csv') as member:
            for row in csv.DictReader(io.TextIOWrapper(member, encoding='ascii', newline='')):
                x.append(int(row['cnt'])); y.append(int(row['registered']))
    cases.append(numeric('bike', x, y, 17379, ['cnt (rentals)', 'registered (rentals)'],
                         'Fanaee-T (2013), UCI, https://doi.org/10.24432/C5W894, CC-BY-4.0'))
    # The official web metadata says 17,389; the pinned hour.csv actually has 17,379 records.
    source = get('cmudict.dict')
    get('cmudict.LICENSE')
    headwords, pronunciations = [], []
    for line in source.read_bytes().splitlines():
        if not line:
            continue
        word, separator, pronunciation = line.partition(b' ')
        if not separator or not word or not pronunciation:
            raise RuntimeError('Unexpected CMU dictionary record')
        headwords.append(word); pronunciations.append(pronunciation)
    string_cases = []
    for name, objects in [('cmu-headwords', headwords), ('cmu-pronunciations', pronunciations)]:
        path = ROOT / f'data/operations/{name}.txt'
        path.write_bytes(b'\n'.join(objects) + b'\n')
        string_cases.append({'name': name, 'objects': len(objects), 'source_corpus': 'cmudict',
                             'extraction': 'First space separates headword from remaining pronunciation field; no normalization',
                             **file_info(path)})
    return {'schema_version': 1, 'sources': {name: {'url': url, 'sha256': sha}
                                            for name, (url, sha) in SOURCES.items()},
            'household_source_sha256': item['sha256'],
            'numeric': cases, 'strings': string_cases,
            'limitations': ['Thresholds use calibration rows; evaluation arrays exclude those rows.',
                'Query schedules are synthetic operations on real, unrepeated data.',
                'CMU headwords/pronunciations are two views of one source corpus.',
                'Bike official site count differs; pinned hour.csv verifies 17,379 records.',
                'Numeric projections retain only named fields, not the complete source datasets.'],
            'protocol': {'repetitions': 9, 'seed': 20261030, 'query_block_rows': 1024,
                         'recipe_tile_rows': 256, 'recipe_cache_tiles': 64,
                         'string_block_bytes': 4096, 'string_queries': 20000,
                         'string_candidate_cap': 64, 'string_max_overlap_bytes': 256}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record-manifest', action='store_true')
    args = parser.parse_args()
    result = build()
    path = ROOT / 'manifests/operations-v1.json'
    if args.record_manifest:
        path.write_text(json.dumps(result, indent=2) + '\n')
    elif result != json.loads(path.read_text()):
        raise RuntimeError('Derived inputs differ from the pinned manifest')
    print('Verified numeric evaluation rows:', {x['name']: x['rows'] for x in result['numeric']})
    print('Verified CMU objects:', {x['name']: x['objects'] for x in result['strings']})


if __name__ == '__main__':
    main()
