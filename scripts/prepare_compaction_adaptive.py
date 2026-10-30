#!/usr/bin/env python3
"""Prepare new real corpora for the adaptive compaction comparison."""
import json
import math
import struct
import zlib
from pathlib import Path

from prepare_compaction import PAGE, digest, find_pinned

ROOT = Path(__file__).resolve().parents[1]
SOURCES = (
    ('household_row', 'data/household/household-row-i32.bin', 'real household measurements; same source as prior column layout', 'manifests/household.json'),
    ('bike', 'data/end-to-end/bike.csv', 'real bike-sharing records', 'manifests/end-to-end-v1.json'),
    ('cmu_pronunciations', 'data/operations/cmu-pronunciations.txt', 'real pronunciation dictionary', 'manifests/operations-v1.json'),
    ('random_control', 'data/initial/random.bin', 'previous synthetic incompressible control', 'manifests/initial-v2.json'),
)


def main():
    destination = ROOT / 'data/compaction-adaptive'
    destination.mkdir(parents=True, exist_ok=True)
    cases = []
    for name, relative, kind, upstream in SOURCES:
        source = ROOT / relative
        if not source.is_file():
            raise SystemExit(f'Missing prior corpus: {relative}')
        source_hash = digest(source)
        expected = set(find_pinned(json.loads((ROOT / upstream).read_text()), relative))
        if expected != {source_hash}:
            raise RuntimeError('Source does not match its pinned prior manifest: ' + relative)
        count = math.ceil(source.stat().st_size / PAGE)
        packed = destination / (name + '.pcmp')
        compressed = 0
        with source.open('rb') as original, packed.open('wb') as output:
            output.write(b'PCMP001\0' + struct.pack('<II', PAGE, count))
            for page in iter(lambda: original.read(PAGE), b''):
                blob = zlib.compress(page, level=1)
                if zlib.decompress(blob) != page:
                    raise RuntimeError('Packed data failed lossless round trip')
                output.write(struct.pack('<II', len(page), len(blob)))
                output.write(blob)
                compressed += len(blob)
        copies = min(256, max(1, math.ceil(48 * 2**20 / compressed)))
        cases.append({'name': name, 'kind': kind,
                      'source': {'path': relative, 'bytes': source.stat().st_size, 'sha256': source_hash},
                      'upstream_manifest': {'path': upstream, 'sha256': digest(ROOT / upstream)},
                      'packed': {'path': str(packed.relative_to(ROOT)), 'bytes': packed.stat().st_size, 'sha256': digest(packed)},
                      'records': count, 'compressed_payload_bytes': compressed, 'copies': copies,
                      'replicated_payload_bytes': compressed * copies})
        print(f'{name}: {count} verified pages; {compressed/2**20:.2f} MiB compressed; {copies} copies')
    manifest = {
        'schema_version': 1, 'experiment': 'adaptive user-space minor compaction',
        'purpose': 'Compare zero-copy-first and a budget-aware rule with the prior policies; no kernel modification.',
        'data_caveat': 'Repeated corpora are not independent data; synthetic deletion layouts are not production traces.',
        'codec': 'zlib level 1', 'zlib_version': zlib.ZLIB_VERSION, 'pool_target_mib': 48,
        'protocol': {
            'policies': ['fifo', 'benefit', 'efficiency', 'tail_first', 'adaptive'],
            'layouts': ['front', 'tail', 'random', 'mixed', 'scrambled', 'rare_tail'],
            'quotas_by_layout': {'front': [0.5], 'tail': [0.5],
                                 'random': [0.25, 0.5, 1.0],
                                 'mixed': [0.25, 0.5, 1.0],
                                 'scrambled': [0.25, 0.5, 1.0],
                                 'rare_tail': [0.25, 0.5, 1.0]},
            'repetitions': 5, 'order_seed': 20261031, 'layout_seed_base': 910000,
            'segment_bytes': 65536, 'cache_scrub_mib': 64,
            'control_repetitions': 1,
            'success_rule': 'Exploratory criterion: adaptive median paired phase time >=10% below FIFO in scrambled 25% and 50% on at least 2 of 3 real sources, and no real-source median regression >10% at scrambled 100%.',
            'primary_timing': 'Native selection + data movement + handles + unmapping; setup and integrity separate.',
        },
        'cases': cases,
    }
    (ROOT / 'manifests/compaction-adaptive-v1.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
