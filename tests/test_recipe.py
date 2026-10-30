#!/usr/bin/env python3
"""Independent tiny integer oracle and accounting checks for experiment B."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest

MASK = (1 << 64) - 1
MISSING = -(1 << 31)
BINARY = None


def mix(value):
    value = (value ^ (value >> 30)) * 0xBF58476D1CE4E5B9 & MASK
    value = (value ^ (value >> 27)) * 0x94D049BB133111EB & MASK
    return value ^ (value >> 31)


def sequential_checksum(xs, ys):
    checksum = 0
    for row, (x, y) in enumerate(zip(xs, ys)):
        values = ([-(1 << 63)] * 8 if MISSING in (x, y) else
                  [x+y, x-y, x*y, x*x, y*y, min(x, y), max(x, y), abs(x-y)])
        for feature, value in enumerate(values):
            checksum = (checksum + mix((value & MASK) ^ ((row * 0x9E3779B97F4A7C15 + feature) & MASK))) & MASK
    return f'{checksum:016x}'


def fnv(data):
    value = 14695981039346656037
    for byte in data:
        value = ((value ^ byte) * 1099511628211) & MASK
    return f'{value:016x}'


class RecipeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='recipe-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        # Both int32 extrema, missingness, mixed signs, and an incomplete last tile.
        self.xs = [MISSING, -(1 << 31)+1, (1 << 31)-1, 0, -7, 13, 90] * 37
        self.ys = [21, (1 << 31)-1, -(1 << 31)+1, MISSING, 31, -17, 0] * 37
        self.x = self.root/'x.bin'
        self.y = self.root/'y.bin'
        self.x.write_bytes(struct.pack(f'<{len(self.xs)}i', *self.xs))
        self.y.write_bytes(struct.pack(f'<{len(self.ys)}i', *self.ys))

    def command(self, *extra):
        return [str(BINARY), '--x', str(self.x), '--y', str(self.y), '--repetitions', '3',
                '--tile-rows', '16', '--cache-tiles', '4', '--queries', '64', *extra]

    def run_json(self, *extra):
        completed = subprocess.run(self.command(*extra), check=True, capture_output=True, text=True)
        return json.loads(completed.stdout)

    def test_exact_values_accounting_and_trials(self):
        report = self.run_json()
        self.assertEqual(report['schema_version'], 1)
        self.assertTrue(report['checksums_verified'])
        self.assertEqual(report['inputs']['rows'], len(self.xs))
        self.assertEqual(report['inputs']['x_fnv1a64_le'], fnv(self.x.read_bytes()))
        self.assertEqual(report['inputs']['y_fnv1a64_le'], fnv(self.y.read_bytes()))
        expected = sequential_checksum(self.xs, self.ys)
        self.assertEqual(report['workloads'][0]['expected_checksum'], expected)
        self.assertEqual(report['workloads'][0]['value_visits'], len(self.xs)*8)
        methods = {method['name']: method for method in report['methods']}
        self.assertEqual(set(methods), {'materialized', 'compressed_blocks', 'recipe', 'recipe_cache', 'recipe_direct'})
        positions = [[] for _ in range(3)]
        for name, method in methods.items():
            memory = method['memory']
            self.assertEqual(memory['source_logical_bytes'], len(self.xs)*8)
            self.assertEqual(memory['retained_accounted_bytes'], sum(memory[key] for key in (
                'source_capacity_bytes', 'derived_capacity_bytes', 'index_capacity_bytes',
                'cache_capacity_bytes', 'scratch_capacity_bytes', 'recipe_descriptor_bytes',
                'other_object_metadata_bytes', 'common_query_schedule_bytes')))
            self.assertGreaterEqual(memory['derived_capacity_bytes'], memory['derived_stored_bytes'])
            self.assertEqual(memory['construction_accounted_peak_bytes'], memory['retained_accounted_bytes'] +
                             memory['construction_extra_scratch_bytes'])
            self.assertEqual(method['workloads'][0]['checksum'], expected)
            self.assertEqual(len(method['construction_samples']), 3)
            for sample in method['construction_samples']:
                positions[sample['repetition']].append(sample['order_position'])
                self.assertGreaterEqual(sample['wall_ms'], 0)
                self.assertGreaterEqual(sample['cpu_ms'], 0)
            for workload in method['workloads']:
                self.assertEqual(len(workload['samples']), 3)
            if name in ('recipe', 'recipe_cache', 'recipe_direct'):
                self.assertEqual(memory['derived_capacity_bytes'], 0)
        for repetition in positions:
            self.assertEqual(sorted(repetition), [0, 1, 2, 3, 4])
        self.assertEqual(methods['materialized']['memory']['derived_stored_bytes'], len(self.xs)*64)
        self.assertEqual(methods['recipe']['memory']['scratch_capacity_bytes'], 16*64)
        self.assertEqual(methods['recipe_cache']['memory']['cache_capacity_bytes'], 4*16*64)
        self.assertEqual(methods['recipe_direct']['memory']['scratch_capacity_bytes'], 0)
        self.assertEqual(methods['recipe_direct']['memory']['cache_capacity_bytes'], 0)
        for workload in methods['recipe_direct']['workloads']:
            for sample in workload['samples']:
                self.assertEqual(sample['tile_hits'], 0)
                self.assertEqual(sample['tile_misses'], 0)
        self.assertEqual(len(report['amortization_vs_materialized']), 16)
        # Absolute paths and raw sample values are not published in the report.
        self.assertNotIn(str(self.root), json.dumps(report))

    def test_only_method_and_output_file(self):
        for name in ('materialized', 'compressed_blocks', 'recipe', 'recipe_cache', 'recipe_direct'):
            output = self.root/f'{name}.json'
            completed = subprocess.run(self.command('--repetitions', '1', '--only-method', name,
                                      '--output', str(output)), check=True, capture_output=True, text=True)
            self.assertEqual(completed.stdout, '')
            report = json.loads(output.read_text())
            self.assertEqual([m['name'] for m in report['methods']], [name])
            self.assertEqual(report['configuration']['only_method'], name)
            self.assertEqual(report['amortization_vs_materialized'], [])
            self.assertEqual(report['workloads'][0]['expected_checksum'], sequential_checksum(self.xs, self.ys))

    def test_single_missing_row(self):
        self.x.write_bytes(struct.pack('<i', MISSING))
        self.y.write_bytes(struct.pack('<i', 1))
        report = self.run_json('--repetitions', '1')
        self.assertEqual(report['workloads'][0]['expected_checksum'], sequential_checksum([MISSING], [1]))

    def test_rejects_malformed_inputs_and_configuration(self):
        for extra in (('--tile-rows', '0'), ('--cache-tiles', '-1'), ('--queries', '0'),
                      ('--only-method', 'unknown'), ('--repetitions', '0')):
            self.assertNotEqual(subprocess.run(self.command(*extra), capture_output=True).returncode, 0)
        for invalid in (b'', b'abc', struct.pack('<i', 7)):
            self.y.write_bytes(invalid)
            self.assertNotEqual(subprocess.run(self.command(), capture_output=True).returncode, 0)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    args, remaining = parser.parse_known_args()
    BINARY = args.binary.resolve()
    unittest.main(argv=[__file__, *remaining])
