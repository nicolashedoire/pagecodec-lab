#!/usr/bin/env python3
"""Independently verify packed inputs and all recorded source fingerprints."""
import argparse
import json
import struct
import zlib
from pathlib import Path

from prepare_compaction import digest

ROOT = Path(__file__).resolve().parents[1]
OFFSET = 14695981039346656037
PRIME = 1099511628211
MASK = (1 << 64) - 1


def fnv(hash_value, values):
    for value in values:
        hash_value = ((hash_value ^ value) * PRIME) & MASK
    return hash_value


def u64(hash_value, value):
    return fnv(hash_value, value.to_bytes(8, 'little'))


def exact(stream, length):
    data = stream.read(length)
    if len(data) != length:
        raise ValueError('Truncated packed input')
    return data


def verify(case):
    for label in ('source', 'packed', 'upstream_manifest'):
        item = case[label]
        if digest(ROOT / item['path']) != item['sha256']:
            raise ValueError('Post-run SHA-256 differs for ' + item['path'])
    with (ROOT/case['source']['path']).open('rb') as source, (ROOT/case['packed']['path']).open('rb') as packed:
        if exact(packed, 8) != b'PCMP001\0':
            raise ValueError('Invalid packed magic')
        page, count = struct.unpack('<II', exact(packed, 8))
        if page != 16384 or count != case['records']:
            raise ValueError('Packed header differs from manifest')
        h = u64(u64(OFFSET, page), count)
        for _ in range(count):
            raw, compressed = struct.unpack('<II', exact(packed, 8))
            if not 0 < raw <= page or not 0 < compressed <= 65536:
                raise ValueError('Invalid packed page sizes')
            blob = exact(packed, compressed)
            if zlib.decompress(blob) != exact(source, raw):
                raise ValueError('Packed page differs from original source bytes')
            h = fnv(u64(u64(h, raw), compressed), blob)
        if source.read(1) or packed.read(1):
            raise ValueError('Unaccounted source or packed input tail')
    return format(h, '016x')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--results', type=Path, required=True)
    args = parser.parse_args()
    location = args.results.resolve()
    report = json.loads((location/'all-results.json').read_text())
    expected = {c['name']: verify(c) for c in report['manifest']['cases']}
    counts = {name: 0 for name in expected}
    for sample in report['samples']:
        name=sample['case']; actual=sample['measurement']['source_fnv1a64']
        if name not in expected or actual != expected[name]:
            raise ValueError('Engine input fingerprint does not match independent packed-byte oracle')
        counts[name] += 1
    result={'schema_version':1, 'all_results_sha256':digest(location/'all-results.json'),
            'audit_script_sha256':digest(Path(__file__).resolve()),
            'post_run_source_packed_and_upstream_sha256_match':True,
            'all_packed_pages_decompress_to_original_source_bytes':True,
            'all_engine_source_fingerprints_match_independent_oracle':True,
            'noncryptographic_fingerprint_is_not_a_cryptographic_integrity_proof':True,
            'engine_fnv1a64_by_case':expected,'sample_counts_by_case':counts}
    (location/'input-audit.json').write_text(json.dumps(result,indent=2)+'\n')
    print('Verified original source, compressed pages and input fingerprints for',sum(counts.values()),'processes.')


if __name__=='__main__':
    main()
