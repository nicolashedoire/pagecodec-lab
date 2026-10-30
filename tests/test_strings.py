#!/usr/bin/env python3
"""Portable correctness/accounting checks for the immutable string experiment."""

import argparse
import json
import math
import random
import subprocess
import tempfile
from pathlib import Path

METHODS = {"concat", "intern", "bounded_overlap", "zlib_blocks"}
OFFSET = 14695981039346656037
PRIME = 1099511628211
MASK = (1 << 64) - 1


def fnv(data):
    state = OFFSET
    for byte in data:
        state = ((state ^ byte) * PRIME) & MASK
    return f"{state:016x}"


def consume_repeated(data, count):
    state = OFFSET
    for _ in range(count):
        for byte in data:
            state = ((state ^ byte) * PRIME) & MASK
        state = ((state ^ len(data)) * PRIME) & MASK
    return f"{state:016x}"


def objects(data):
    if not data:
        return []
    rows = data.split(b"\n")
    return rows[:-1] if data.endswith(b"\n") else rows


def check(report, data, trials, names=METHODS):
    rows = objects(data)
    unique = set(rows)
    assert report["schema_version"] == 1
    assert report["experiment"] == "immutable_strings"
    assert report["input_bytes"] == len(data)
    assert report["input_fnv1a64"] == fnv(data)
    assert report["object_count"] == len(rows)
    assert report["total_object_bytes"] == sum(map(len, rows))
    assert report["max_object_bytes"] == max(map(len, rows), default=0)
    assert report["source_oracle_retained_during_lookup"] is False
    assert report["trials"] == trials
    assert report["queries_per_workload"] == (201 if rows else 0)
    assert len(report["trial_order"]) == trials
    for order in report["trial_order"]:
        assert len(order) == len(names) and set(order) == names
    assert len(report["workload_order"]) == trials * len(names)
    for order in report["workload_order"]:
        assert len(order) == 5
        assert set(order[1:]) == {"uniform.view", "uniform.copy", "hot.view", "hot.copy"}
    methods = {entry["method"]: entry for entry in report["methods"]}
    assert set(methods) == names
    workload_checksums = {"uniform": set(), "hot": set()}
    for name, method in methods.items():
        assert method["verified_all_objects"] is True
        assert method["index_bytes"] == 16 * len(rows)
        assert method["block_metadata_bytes"] == method["block_count"] * 32
        assert method["logical_storage_bytes"] == (
            method["payload_bytes"] + method["index_bytes"] + method["block_metadata_bytes"]
        )
        assert method["capacity_storage_bytes"] >= method["logical_storage_bytes"]
        assert method["resident_view_capacity_bytes"] == (
            method["capacity_storage_bytes"] + method["lookup_workspace_view_bytes"]
        )
        assert method["resident_copy_capacity_bytes"] == (
            method["capacity_storage_bytes"] + method["lookup_workspace_copy_bytes"]
        )
        assert method["lookup_workspace_copy_bytes"] >= (
            method["lookup_workspace_view_bytes"] + max(map(len, rows), default=0)
        )
        assert method["construction_estimated_peak_bytes"] >= method["capacity_storage_bytes"]
        assert method["raw_fallback_blocks"] <= method["block_count"]
        assert len(method["construction_samples_ms"]) == trials
        assert all(math.isfinite(x) and x >= 0 for x in method["construction_samples_ms"])
        if name == "concat":
            assert method["payload_bytes"] == sum(map(len, rows))
        if name in {"intern", "bounded_overlap"}:
            assert method["unique_objects"] == len(unique)
            assert method["payload_bytes"] <= sum(map(len, unique))
        if name == "intern":
            assert method["payload_bytes"] == sum(map(len, unique))
        if name == "bounded_overlap":
            assert method["payload_bytes"] + method["overlap_saved_bytes"] == sum(map(len, unique))
            assert method["overlap_candidate_checks"] <= len(unique) * report["candidate_cap"]
        if name != "zlib_blocks":
            assert method["block_metadata_bytes"] == 0
            assert method["lookup_workspace_view_bytes"] == 0
        else:
            assert method["payload_bytes"] <= sum(map(len, rows))
        for workload_name, workload in method["workloads"].items():
            assert workload_name in workload_checksums
            assert set(workload) == {"view", "copy"}
            for mode in workload.values():
                samples = mode["ns_per_query_samples"]
                assert len(samples) == trials
                assert all(math.isfinite(x) and x >= 0 for x in samples)
                workload_checksums[workload_name].add(mode["checksum_fnv1a64"])
                if len(rows) == 1:
                    assert mode["checksum_fnv1a64"] == consume_repeated(rows[0], 201)
                if not rows:
                    assert mode["checksum_fnv1a64"] == f"{OFFSET:016x}"
                    assert samples == [0] * trials
    assert all(len(values) == 1 for values in workload_checksums.values())
    return methods


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    rng = random.Random(1701)
    random_rows = [
        bytes(rng.choice([x for x in range(256) if x != 10]) for _ in range(length))
        for length in [1, 7, 8, 31, 127, 128, 129, 500]
    ]
    fixtures = {
        "empty": b"",
        "one_empty": b"\n",
        "empty_last_object": b"alpha\n\n",
        "unterminated": b"\x00single\r\xc3\xa9",
        "exact_bytes": b"\r\nalpha \nalpha\n\x00alpha\x00\n\xc3\xa9\nalpha \n\n",
        "overlap": b"abcdefgh12345678\n12345678ijklmnop\nijklmnopQRSTUVWX\n12345678ijklmnop\n",
        "short_no_overlap": b"abc\nbcd\ncde\nabc\n",
        "compressed_oversized": b"A" * 6000 + b"\n" + b"A" * 6000 + b"\n" + b"xyz" * 100 + b"\n",
        "raw_fallback": b"\n".join(random_rows),
    }
    with tempfile.TemporaryDirectory(prefix="string-store-tests-") as directory:
        directory = Path(directory)
        for name, content in fixtures.items():
            path = directory / (name + ".txt")
            path.write_bytes(content)
            trials = 9 if name == "overlap" else 2
            command = [
                str(binary), "--input", str(path), "--trials", str(trials), "--queries", "201",
                "--seed", "4129", "--block-bytes", "128", "--candidate-cap", "64",
                "--max-overlap-bytes", "256",
            ]
            run = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            report = json.loads(run.stdout)
            methods = check(report, content, trials)
            if name == "overlap":
                assert methods["bounded_overlap"]["overlap_saved_bytes"] >= 16
                assert methods["bounded_overlap"]["payload_bytes"] < methods["intern"]["payload_bytes"]
            if name == "short_no_overlap":
                assert methods["bounded_overlap"]["payload_bytes"] == methods["intern"]["payload_bytes"]
            if name == "compressed_oversized":
                assert methods["zlib_blocks"]["raw_fallback_blocks"] < methods["zlib_blocks"]["block_count"]
                assert methods["zlib_blocks"]["lookup_workspace_view_bytes"] >= 6000
            if name == "raw_fallback":
                assert methods["zlib_blocks"]["raw_fallback_blocks"] > 0
        path = directory / "isolated.txt"
        content = b"abcdefgh12345678\n12345678ijklmnop\nabcdefgh12345678\n"
        path.write_bytes(content)
        for method in sorted(METHODS):
            output = directory / f"{method}.json"
            run = subprocess.run(
                [str(binary), "--input", str(path), "--output", str(output),
                 "--trials", "1", "--queries", "201", "--only-method", method],
                check=True, capture_output=True, text=True, timeout=30,
            )
            assert run.stdout == ""
            check(json.loads(output.read_text()), content, 1, {method})
        for flag, value in [("--trials", "0"), ("--queries", "-1"), ("--candidate-cap", "0"), ("--only-method", "unknown")]:
            run = subprocess.run([str(binary), "--input", str(path), flag, value], capture_output=True, text=True)
            assert run.returncode != 0 and "string_bench:" in run.stderr
    print("PASS: 9 byte-exact fixtures, 4 isolated methods, accounting, paired checksums, overlap/no-gain cases and invalid options.")


if __name__ == "__main__":
    main()
