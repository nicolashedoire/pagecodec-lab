#!/usr/bin/env python3
"""Independent byte/handle oracle and mapping arithmetic for minor compaction."""
import argparse
from fractions import Fraction
import json
import math
import os
from pathlib import Path
import random
import struct
import subprocess
import tempfile
import unittest
import zlib

BINARY = None
PAGE = os.sysconf('SC_PAGE_SIZE')
POLICIES = ('none', 'fifo', 'benefit', 'efficiency')
MASK = (1 << 64)-1


def align(n, unit):
    return ((n+unit-1)//unit)*unit


def fnv(data, value=14695981039346656037):
    for byte in data:
        value = ((value ^ byte)*1099511628211) & MASK
    return value


def payloads(sizes, seed=20261030):
    rng = random.Random(seed)
    return [(n, zlib.compress(bytes(rng.randrange(256) for _ in range(n)), level=1)) for n in sizes]


def expected(records, policy, layout, fraction, copies=1, segment_bytes=65536):
    segments = []
    for handle, (raw, data) in enumerate(records*copies):
        span = align(len(data), 16)
        if not segments or segments[-1]['used']+span > segment_bytes:
            segments.append({'used': 0, 'records': []})
        s = segments[-1]
        s['records'].append({'handle': handle, 'data': data, 'offset': s['used'], 'span': span, 'live': True})
        s['used'] += span
    before = maximum = initial_trimmed = empty = moved = moved_records = payload_moved = 0
    checksum = 14695981039346656037
    candidates = []
    for index, s in enumerate(segments):
        s['mapped'] = align(s['used'], PAGE)
        initial_trimmed += segment_bytes-s['mapped']
        ratio, direction = .45, layout
        if layout == 'none':
            ratio = 0
        if layout == 'mixed':
            ratio = (0, .15, .35, .65)[index % 4]
            direction = 'front' if (index+index//4) % 2 else 'tail'
        order = list(range(len(s['records'])))
        if direction == 'tail':
            order.reverse()
        deleted = 0
        for i in order:
            if deleted >= math.floor(s['used']*ratio):
                break
            s['records'][i]['live'] = False
            deleted += s['records'][i]['span']
        live = s['used']-deleted
        if not live:
            empty += s['mapped']
            continue
        before += s['mapped']
        destination = would_move = would_records = would_payload = 0
        for r in s['records']:
            if r['live']:
                checksum = fnv(struct.pack('<QQ', r['handle'], len(r['data']))+r['data'], checksum)
                if r['offset'] != destination:
                    would_move += r['span']; would_records += 1; would_payload += len(r['data'])
                destination += r['span']
        gain = s['mapped']-align(live, PAGE) if deleted and s['mapped']-live >= PAGE else 0
        maximum += gain
        if gain:
            candidates.append((index, gain, would_move, would_records, would_payload))
    target_pages = math.floor(maximum//PAGE*fraction)
    if maximum and not target_pages:
        target_pages = 1
    target = target_pages*PAGE
    if policy == 'benefit':
        candidates.sort(key=lambda c: (-c[1], c[0]))
    elif policy == 'efficiency':
        candidates.sort(key=lambda c: (-Fraction(c[1], c[2]+PAGE), c[0]))
    elif policy == 'none':
        candidates = []
    released = compacted = 0
    for _, gain, move, count, actual in candidates:
        if released >= target:
            break
        released += gain; moved += move; moved_records += count; payload_moved += actual; compacted += 1
    return {'mapped_before': before, 'mapped_after': before-released, 'max_reclaimable_bytes': maximum,
            'target_bytes': target, 'released_bytes': released, 'bytes_moved': moved,
            'records_moved': moved_records, 'payload_bytes_moved': payload_moved,
            'segments_compacted': compacted, 'initial_tail_trimmed_bytes': initial_trimmed,
            'common_empty_released_bytes': empty, 'checksum_before': f'{checksum:016x}',
            'checksum_after': f'{checksum:016x}', 'target_met': released >= target}


class CompactionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='compaction-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def packed(self, records, logical=16384):
        path = self.root/'records.pcmp'
        with path.open('wb') as out:
            out.write(b'PCMP001\0'+struct.pack('<II', logical, len(records)))
            for raw, data in records:
                out.write(struct.pack('<II', raw, len(data)))
                out.write(data)
        return path

    def run_case(self, records, layout, fraction=.5, copies=1, logical=16384, oracle=True):
        source = self.packed(records, logical)
        rows = {}
        for policy in POLICIES:
            command = [str(BINARY), '--input', str(source), '--policy', policy, '--layout', layout,
                       '--reclaim-fraction', str(fraction), '--copies', str(copies), '--seed', '12345']
            run = subprocess.run(command, check=True, text=True, capture_output=True)
            self.assertEqual(run.stderr, '')
            row = json.loads(run.stdout)
            self.assertTrue(row['checksum_ok'])
            self.assertEqual(row['checksum_before'], row['checksum_after'])
            self.assertEqual(row['source_record_count'], len(records))
            self.assertEqual(row['total_record_count'], len(records)*copies)
            self.assertEqual(row['source_logical_bytes'], sum(r for r, _ in records))
            self.assertEqual(row['source_compressed_bytes'], sum(len(d) for _, d in records))
            self.assertEqual(row['live_record_count']+row['deleted_record_count'], row['total_record_count'])
            self.assertEqual(row['live_payload_bytes']+row['deleted_payload_bytes'], row['source_compressed_bytes']*copies)
            self.assertEqual(row['mapped_before']-row['mapped_after'], row['released_bytes'])
            self.assertEqual(row['initial_mapped_bytes']-row['common_empty_released_bytes'], row['mapped_before'])
            self.assertEqual(row['page_bytes'], PAGE)
            self.assertEqual(row['released_bytes'] % PAGE, 0)
            self.assertEqual(row['overshoot_bytes'], max(0, row['released_bytes']-row['target_bytes']))
            for key in ('setup_ms', 'target_scan_ms', 'checksum_ms', 'elapsed_ms', 'conditioning_ms', 'cpu_ms'):
                self.assertTrue(math.isfinite(row[key]))
                self.assertGreaterEqual(row[key], 0)
            self.assertIsNone(row['compaction_ms'])
            self.assertIsNone(row['reclaim_ms'])
            if policy in ('fifo', 'none'):
                self.assertIsNone(row['decision_ms'])
            else:
                self.assertGreaterEqual(row['decision_ms'], 0)
            if policy == 'none' or row['target_bytes'] == 0:
                self.assertEqual(row['released_bytes'], 0)
                self.assertEqual(row['bytes_moved'], 0)
            else:
                self.assertTrue(row['target_met'])
            if policy == 'fifo':
                self.assertEqual(row['decision_metadata_bytes'], 0)
            if oracle:
                for key, value in expected(records, policy, layout, fraction, copies).items():
                    self.assertEqual(row[key], value, (policy, layout, key))
            rows[policy] = row
        for key in ('checksum_before', 'checksum_after', 'target_bytes', 'max_reclaimable_bytes',
                    'mapped_before', 'eligible_count', 'live_record_count', 'deleted_record_count'):
            self.assertEqual(len({r[key] for r in rows.values()}), 1, key)
        return rows

    def test_front_tail_and_no_holes_against_byte_oracle(self):
        records = payloads([1000]*100)
        for layout in ('front', 'tail', 'none'):
            for fraction in (.25, .5, 1):
                rows = self.run_case(records, layout, fraction)
                if layout == 'tail':
                    self.assertTrue(all(r['bytes_moved'] == 0 for r in rows.values()))
                if layout == 'none':
                    self.assertTrue(all(r['target_bytes'] == 0 for r in rows.values()))

    def test_mixed_variable_records_and_full_quota_work_invariance(self):
        records = payloads([211, 499, 1111, 2257, 3019, 5101]*35)
        for fraction in (.25, .5, 1):
            rows = self.run_case(records, 'mixed', fraction, copies=2)
            if fraction == 1:
                for key in ('bytes_moved', 'records_moved', 'released_bytes', 'segments_compacted'):
                    self.assertEqual(len({rows[p][key] for p in POLICIES if p != 'none'}), 1)

    def test_random_integrity_and_full_quota(self):
        records = payloads([999, 2103, 4097, 6311]*32)
        rows = self.run_case(records, 'random', 1, oracle=False)
        for key in ('bytes_moved', 'records_moved', 'released_bytes'):
            self.assertEqual(len({rows[p][key] for p in POLICIES if p != 'none'}), 1)

    def test_small_subpage_and_empty_segments(self):
        self.run_case(payloads([30]*5), 'front')
        rows = self.run_case(payloads([200, 400, 1000, 2000]*20), 'subpage', oracle=False)
        self.assertTrue(all(r['max_reclaimable_bytes'] == 0 for r in rows.values()))
        empty = self.run_case(payloads([50000]), 'front', logical=65536)
        self.assertTrue(all(r['mapped_before'] == 0 and r['common_empty_released_bytes'] > 0 for r in empty.values()))

    def test_malformed_packed_input_and_options(self):
        source = self.packed(payloads([100]))
        command = [str(BINARY), '--input', str(source)]
        conditioned = json.loads(subprocess.check_output(command+['--cache-scrub-mib', '1'], text=True))
        self.assertEqual(conditioned['conditioning_bytes'], 1024*1024)
        self.assertTrue(conditioned['checksum_ok'])
        for extra in (['--copies', '0'], ['--copies', '1025'], ['--policy', 'bad'], ['--layout', 'bad'],
                      ['--reclaim-fraction', '0'], ['--reclaim-fraction', 'nan'], ['--reclaim-fraction', '1.1'],
                      ['--segment-bytes', str(PAGE+1)], ['--cache-scrub-mib', '129']):
            self.assertNotEqual(subprocess.run(command+extra, capture_output=True).returncode, 0)
        header = b'PCMP001\0'+struct.pack('<II', 16384, 1)
        for data in (b'', b'badmagic'+b'0'*8, header, header+struct.pack('<II', 0, 1)+b'x', header+struct.pack('<II', 16385, 1)+b'x',
                     header+struct.pack('<II', 1, 65537)+b'x', header+struct.pack('<II', 1, 0),
                     header+struct.pack('<II', 1, 2)+b'x', header+struct.pack('<II', 1, 1)+b'xy'):
            source.write_bytes(data)
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    args, remaining = parser.parse_known_args()
    BINARY = args.binary.resolve()
    unittest.main(argv=[__file__, *remaining])
