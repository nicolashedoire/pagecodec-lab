#!/usr/bin/env python3
"""Independent integer oracle for text-to-report pipelines, including the offline sweep."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
import tempfile
import unittest

MISSING = -(1 << 31)
METHODS = ('raw', 'bitplane', 'bitplane_adaptive', 'sorted_blocks', 'stream_sweep')
BINARY = None


def exact_decimal(value):
    if value == MISSING:
        return '?'
    return ('-' if value < 0 else '') + f'{abs(value)//1000}.{abs(value)%1000:03d}'


def oracle(xs, ys, queries):
    result = []
    for begin, end, threshold in queries:
        selected = [i for i in range(begin, end) if xs[i] != MISSING and xs[i] >= threshold]
        result.append({'begin': begin, 'end': end, 'threshold': threshold,
                       'count': len(selected), 'sum': sum(ys[i] for i in selected if ys[i] != MISSING)})
    return result


def fingerprint(xs, ys):
    result = 14695981039346656037
    for pair in zip(xs, ys):
        for byte in struct.pack('<ii', *pair):
            result = ((result ^ byte) * 1099511628211) & ((1 << 64) - 1)
    return str(result)


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='pagecodec-report-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def source(self, xs, ys, format_name, crlf=False):
        path = self.root / (format_name + '.csv')
        lines = []
        if format_name == 'household':
            lines.append('Date;Time;Global_active_power;Global_reactive_power;Voltage;Global_intensity;Sub_metering_1;Sub_metering_2;Sub_metering_3')
            lines += [f'1/1/2000;00:00:00;{exact_decimal(x)};{exact_decimal(y)};230;0;0;0;0' for x, y in zip(xs, ys)]
        elif format_name == 'bike':
            lines.append('instant,dteday,season,yr,mnth,hr,holiday,weekday,workingday,weathersit,temp,atemp,hum,windspeed,casual,registered,cnt')
            for x, y in zip(xs, ys):
                row = ['0'] * 17
                row[15] = '?' if y == MISSING else str(y)
                row[16] = '?' if x == MISSING else str(x)
                lines.append(','.join(row))
        else:
            for x, y in zip(xs, ys):
                row = ['0'] * 55
                row[0] = '?' if x == MISSING else str(x)
                row[5] = '?' if y == MISSING else str(y)
                lines.append(','.join(row))
        sep = '\r\n' if crlf else '\n'
        path.write_bytes((sep.join(lines) + sep).encode())
        return path

    def plan(self, queries):
        path = self.root / 'queries.tsv'
        path.write_text(''.join(f'{a}\t{b}\t{t}\n' for a, b, t in queries))
        return path

    def command(self, source, format_name, method, rows, skip, queries, output):
        return [str(BINARY), '--input', str(source), '--format', format_name, '--method', method,
                '--rows', str(rows), '--skip-rows', str(skip), '--queries', str(queries), '--output', str(output)]

    def check(self, xs, ys, queries, format_name='covertype', skip=0, crlf=False):
        source = self.source(xs, ys, format_name, crlf)
        plan = self.plan(queries)
        expected = oracle(xs[skip:], ys[skip:], queries)
        reports = {}
        for method in METHODS:
            output = self.root / (method + '.json')
            subprocess.run(self.command(source, format_name, method, len(xs)-skip, skip, plan, output),
                           check=True, capture_output=True, text=True)
            report = json.loads(output.read_text())
            self.assertEqual(report['answers'], expected, method)
            self.assertEqual(report['evaluation_rows'], len(xs)-skip)
            self.assertEqual(report['source_rows'], len(xs))
            self.assertEqual(report['source_bytes'], source.stat().st_size)
            self.assertEqual(report['projected_fnv1a64'], fingerprint(xs[skip:], ys[skip:]))
            self.assertEqual(report['sweep_queries_fused_into_ingestion'], method == 'stream_sweep')
            self.assertEqual(report['query_count'], len(queries))
            self.assertGreater(report['memory']['store_capacity_bytes'], 0)
            self.assertGreaterEqual(report['process_peak_rss_bytes'], report['rss_after_ingestion_bytes'])
            self.assertNotIn(str(self.root), output.read_text())
            reports[method] = report
        return reports

    def test_all_formats_missing_extremes_and_partial_blocks(self):
        rng = random.Random(20261030)
        values = [MISSING, MISSING+1, (1 << 31)-1, -65536, -7, 0, 1, 17, 65536]
        xs = [rng.choice(values) for _ in range(2056)]
        ys = [rng.choice(values) for _ in xs]
        n, skip = 2049, 7
        queries = [(0, n, t) for t in values]
        queries += [(0, 0, 0), (n, n, 0), (63, 1025, -7), (1024, 2048, MISSING), (2048, 2049, 0)]
        for _ in range(64):
            begin = rng.randrange(n+1)
            queries.append((begin, rng.randrange(begin, n+1), rng.choice(values)))
        for format_name in ('covertype', 'household', 'bike'):
            self.check(xs, ys, queries, format_name, skip, crlf=format_name == 'household')

    def test_all_residual_widths(self):
        # Each 1024-row block spans exactly one unsigned residual width, 0..32.
        xs, ys, queries = [], [], []
        for bits in range(33):
            low = -2147483647 if bits == 32 else -123
            high = low if bits == 0 else low + (1 << (bits-1))
            a = [low, high, MISSING, low+1 if bits else low] * 256
            b = list(reversed(a))
            begin = len(xs)
            xs.extend(a); ys.extend(b)
            for threshold in (MISSING, low, high):
                queries.append((begin, begin+1024, threshold))
            queries.append((begin+63, begin+997, high))
        self.check(xs, ys, queries)

    def test_empty_matches_all_missing_constant_and_duplicate_events(self):
        for xs, ys in [([MISSING]*1025, [1]*1025), ([7]*1025, [MISSING]*1025),
                       ([7]*1025, [-31]*1025), ([MISSING], [MISSING])]:
            n = len(xs)
            queries = [(0, n, 0), (0, n, 7), (0, n, 8), (0, n, MISSING),
                       (0, 0, 7), (n, n, 7), (0, n, 7)]
            self.check(xs, ys, queries)

    def test_sweep_plan_prefix_consistency(self):
        xs, ys = list(range(-10, 10)), list(range(10, -10, -1))
        queries = [(0, 20, 0), (2, 17, -2), (5, 10, 3), (0, 20, -8)]
        full = self.check(xs, ys, queries)
        for count in (1, 2):
            short = self.check(xs, ys, queries[:count])
            for method in METHODS:
                self.assertEqual(short[method]['answers'], full[method]['answers'][:count])

    def test_invalid_configuration_source_and_plan(self):
        source = self.source([1, 2], [3, 4], 'covertype')
        plan = self.plan([(0, 2, 0)])
        output = self.root / 'invalid.json'
        base = self.command(source, 'covertype', 'raw', 2, 0, plan, output)
        for key, value in (('--rows', '0'), ('--rows', '1'), ('--rows', '3'), ('--skip-rows', '1'),
                           ('--format', 'unknown'), ('--method', 'unknown')):
            cmd = base.copy();cmd[cmd.index(key)+1] = value
            self.assertNotEqual(subprocess.run(cmd, capture_output=True).returncode, 0)
        for invalid in ('', '0\t3\t1\n', '2\t1\t0\n', '0\t2\t2147483648\n', '0 2 1\n', '0\t2\t\n', '0\t2\t?\n'):
            plan.write_text(invalid)
            self.assertNotEqual(subprocess.run(base, capture_output=True).returncode, 0)
        plan = self.plan([(0, 2, 0)])
        source.write_text('1,2,3\n1,2,3\n')
        self.assertNotEqual(subprocess.run(base, capture_output=True).returncode, 0)
        source = self.source([1, 2], [3, 4], 'household')
        source.write_text(source.read_text().replace('0.001;', '0.0001;'))
        cmd = self.command(source, 'household', 'bitplane', 2, 0, plan, output)
        self.assertNotEqual(subprocess.run(cmd, capture_output=True).returncode, 0)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    args, remaining = parser.parse_known_args()
    BINARY = args.binary.resolve()
    unittest.main(argv=[__file__, *remaining])
