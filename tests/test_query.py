"""Independent integer oracle and edge cases for the standalone query experiment."""
import argparse
import json
import os
from pathlib import Path
import random
import shutil
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
MISSING = -(2**31)
METHODS = {"raw", "for", "bitplane_eager", "bitplane_progressive", "bitplane_adaptive", "rle"}
BINARY = None


class QueryExperimentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="pagecodec-query-test-")
        if BINARY is not None:
            cls.binary = BINARY
            return
        compiler = os.environ.get("CXX") or shutil.which("clang++") or shutil.which("g++")
        if not compiler:
            raise unittest.SkipTest("C++17 compiler unavailable")
        cls.binary = Path(cls.temp.name) / "query-bench"
        subprocess.run([compiler, "-std=c++17", "-O3", "-Wall", "-Wextra", "-Werror",
                        str(ROOT / "experiments/query/query_bench.cpp"), "-o", str(cls.binary)], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def run_case(self, x, y, *, only=None, timed=False):
        directory = Path(self.temp.name)
        xp, yp = directory / "x.bin", directory / "y.bin"
        xp.write_bytes(struct.pack(f"<{len(x)}i", *x))
        yp.write_bytes(struct.pack(f"<{len(y)}i", *y))
        command = [str(self.binary), "--x", str(xp), "--y", str(yp),
                   "--thresholds", f"{MISSING},-1,0,1,{2**31-1}", "--seed", "112358", "--repetitions", "1"]
        if not timed:
            command.append("--validate-only")
        if only:
            command.extend(["--only-method", only])
        result = json.loads(subprocess.check_output(command, text=True))
        self.assertTrue(result["verified"])
        self.assertTrue(result["representation_roundtrips_verified"])
        self.assertGreater(result["validation_cases"], 0)
        expected_methods = {only} if only else METHODS
        self.assertEqual({m["method"] for m in result["methods"]}, expected_methods)
        for workload in result["workloads"]:
            for query in workload["queries"]:
                selected = [i for i in range(query["begin"], query["end"])
                            if x[i] != MISSING and x[i] >= workload["threshold"]]
                self.assertEqual(query["count"], len(selected))
                self.assertEqual(query["sum"], sum(y[i] for i in selected if y[i] != MISSING))
            for entry in workload["results"]:
                self.assertTrue(entry["verified"])
                self.assertEqual(len(entry["samples_ms"]), 1 if timed else 0)
            if timed:
                self.assertEqual(set(workload["trial_order"][0]), expected_methods)
        for entry in result["methods"]:
            self.assertEqual(entry["representation_bytes"], entry["payload_bytes"] + entry["metadata_bytes"])
            self.assertGreaterEqual(entry["allocated_capacity_bytes"], entry["representation_bytes"])
            self.assertEqual(entry["owned_working_bytes"], entry["allocated_capacity_bytes"] + entry["explicit_scratch_bytes"])
        return result

    def test_extreme_signed_values_partial_words_and_blocks(self):
        rng = random.Random(314159)
        values = [MISSING, MISSING+1, -10000001, -1, 0, 1, 7, 65535, 2**30, 2**31-1]
        x = [rng.choice(values) for _ in range(2113)]
        y = [rng.choice(values) for _ in x]
        self.run_case(x, y)

    def test_zero_width_and_missing_y_do_not_change_count(self):
        self.run_case([7] * 1025, [MISSING] * 1025)
        self.run_case([MISSING] * 65, [-12] * 65)
        self.run_case([-5] * 67, [-10] * 67)

    def test_single_row_and_empty_matches(self):
        for x, y in [(MISSING, 1), (0, MISSING), (2**31-1, MISSING+1)]:
            self.run_case([x], [y])

    def test_rle_runs_cross_windows_and_blocks(self):
        x = [-20]*1200 + [MISSING]*31 + [17]*1100 + [0]*9
        y = [-9]*1199 + [18]*20 + [MISSING]*100 + [2]*(len(x)-1319)
        self.run_case(x, y)

    def test_every_residual_width(self):
        rng = random.Random(271828)
        x, y = [], []
        for width in range(33):
            base = MISSING+1 if width == 32 else -(2**30)
            maximum = min((1 << width)-1, 2**31-1-base)
            block = [base, base+maximum]
            block.extend(base+rng.randrange(maximum+1) for _ in range(1022))
            x.extend(block)
            y.extend(reversed(block))
        self.run_case(x, y)

    def test_isolated_method_releases_oracle_arrays_and_times(self):
        x = [i % 31 - 15 for i in range(1077)]
        y = [MISSING if i % 37 == 0 else i*31-40000 for i in range(len(x))]
        for method in sorted(METHODS):
            result = self.run_case(x, y, only=method, timed=True)
            self.assertEqual(result["input_arrays_released_before_timing"], method != "raw")
            self.assertEqual(result["input_arrays_retained_capacity_bytes"] == 0, method != "raw")

    def test_invalid_size_and_method_rejected(self):
        path = Path(self.temp.name) / "bad.bin"
        path.write_bytes(b"abc")
        for extra in [[], ["--only-method", "nonexistent"]]:
            p = subprocess.run([str(self.binary), "--x", str(path), "--y", str(path), *extra], capture_output=True)
            self.assertNotEqual(p.returncode, 0)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    args, remaining = parser.parse_known_args()
    BINARY = args.binary.resolve() if args.binary else None
    unittest.main(argv=[__file__, *remaining])
