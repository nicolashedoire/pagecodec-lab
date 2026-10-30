#!/usr/bin/env python3
"""Pack real corpus pages for the isolated native compaction experiment."""
import argparse
import hashlib
import json
import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PAGE = 16384
SOURCES = (
    ('household', 'data/household/household-column-i32.bin', 'real numeric corpus', 'manifests/household.json'),
    ('covertype', 'data/operations/covertype-x.i32', 'real numeric corpus', 'manifests/operations-v1.json'),
    ('rea_source', 'data/initial/rea_cli_public.js', 'real public JavaScript source', 'manifests/initial-v2.json'),
    ('random_control', 'data/initial/random.bin', 'synthetic incompressible control', 'manifests/initial-v2.json'),
)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def find_pinned(value, path):
    if isinstance(value, dict):
        found = [value['sha256']] if value.get('path') == path and 'sha256' in value else []
        return found + [h for child in value.values() for h in find_pinned(child, path)]
    if isinstance(value, list):
        return [h for child in value for h in find_pinned(child, path)]
    return []


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pool-mib', type=int, default=96)
    args = parser.parse_args()
    if not 1 <= args.pool_mib <= 128:
        parser.error('pool-mib must be between 1 and 128')
    destination = ROOT / 'data/compaction'
    destination.mkdir(parents=True, exist_ok=True)
    cases = []
    for name, relative, kind, upstream in SOURCES:
        source = ROOT / relative
        if not source.is_file():
            raise SystemExit(f'Missing existing corpus {relative}; see docs/DATASETS.md')
        source_hash = digest(source)
        expected = set(find_pinned(json.loads((ROOT / upstream).read_text()), relative))
        if expected != {source_hash}:
            raise RuntimeError(f'Existing corpus disagrees with its pinned upstream manifest: {relative}')
        count = math.ceil(source.stat().st_size / PAGE)
        packed = destination / (name + '.pcmp')
        compressed_bytes = 0
        with source.open('rb') as original, packed.open('wb') as output:
            output.write(b'PCMP001\0' + struct.pack('<II', PAGE, count))
            for page in iter(lambda: original.read(PAGE), b''):
                blob = zlib.compress(page, level=1)
                if zlib.decompress(blob) != page:
                    raise RuntimeError('Lossless packing verification failed')
                output.write(struct.pack('<II', len(page), len(blob)))
                output.write(blob)
                compressed_bytes += len(blob)
        copies = min(256, max(1, math.ceil(args.pool_mib * 2**20 / compressed_bytes)))
        cases.append({
            'name': name, 'kind': kind,
            'source': {'path': relative, 'bytes': source.stat().st_size, 'sha256': source_hash},
            'upstream_manifest': {'path': upstream, 'sha256': digest(ROOT / upstream)},
            'packed': {'path': str(packed.relative_to(ROOT)), 'bytes': packed.stat().st_size,
                       'sha256': digest(packed)},
            'logical_page_bytes': PAGE, 'records': count,
            'compressed_payload_bytes': compressed_bytes, 'copies': copies,
            'replicated_payload_bytes': compressed_bytes * copies,
            'note': 'Copies repeat the same corpus; they are not independent real-world data.',
        })
        print(f'{name}: {count} verified pages, {compressed_bytes/2**20:.2f} MiB compressed, {copies} copies')
    manifest = {
        'schema_version': 1, 'codec': 'zlib level 1', 'zlib_version': zlib.ZLIB_VERSION,
        'purpose': 'Real compressed payloads; synthetic deletion layouts; native minor compaction only.',
        'pool_target_mib': args.pool_mib,
        'protocol': {
            'policies': ['fifo', 'benefit', 'efficiency'],
            'layouts': ['none', 'subpage', 'front', 'tail', 'random', 'mixed'],
            'quotas_by_layout': {'none': [0.5], 'subpage': [0.5], 'front': [0.5],
                                'tail': [0.5], 'random': [0.25, 0.5, 1.0],
                                'mixed': [0.25, 0.5, 1.0]},
            'repetitions': 9, 'order_seed': 20261030, 'layout_seed_base': 84000,
            'segment_bytes': 65536,
            'cache_scrub_mib': 64,
            'none_control_repetitions': 1,
            'primary_timing': 'Native decision + memmove + handle update + munmap phase.',
            'setup_and_validation': 'Reported separately; not a complete application speedup.',
            'selection_rule': 'Freeze all policies before timing; no final-run parameter tuning.',
        },
        'cases': cases,
    }
    path = ROOT / 'manifests/compaction-v1.json'
    path.write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
