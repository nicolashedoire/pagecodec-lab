#!/usr/bin/env python3
"""Check comparator structure and accounting; this is not a performance test."""
import argparse
import json
import math
import random
import struct
import subprocess
import tempfile
from pathlib import Path

METHODS = {"raw", "apple_lz4", "apple_lz4_raw", "apple_lzfse", "custom_rc"}
PAGE_SIZE = 16 * 1024
TOP_LEVEL_FIELDS = {
    "schema_version", "input_path", "input_bytes", "page_size", "page_count",
    "repetitions", "inner_iterations", "warmup_rounds_per_method", "shuffle_seed",
    "checksum_fnv1a64", "timing_scope", "storage_scope", "apple_selection",
    "custom_selection", "trial_order", "results",
}
RESULT_FIELDS = {
    "method", "payload_bytes", "metadata_bytes", "total_storage_bytes",
    "raw_pages", "compressed_pages", "encode_ms", "decode_ms",
    "encode_samples_ms", "decode_samples_ms", "verified",
}


def fnv1a64(data):
    value = 14695981039346656037
    for byte in data:
        value = ((value ^ byte) * 1099511628211) & ((1 << 64) - 1)
    return f"{value:016x}"


def check_result(result, data, input_path):
    size = len(data)
    pages = (size + PAGE_SIZE - 1) // PAGE_SIZE
    # The public report contains measurements and a checksum, never input bytes.
    assert set(result) == TOP_LEVEL_FIELDS
    assert result["schema_version"] == 1
    assert result["input_path"] == str(input_path)
    assert result["shuffle_seed"] == 4129
    assert result["input_bytes"] == size
    assert result["page_size"] == PAGE_SIZE
    assert result["page_count"] == pages
    assert result["repetitions"] == 1
    assert result["warmup_rounds_per_method"] == 1
    assert result["checksum_fnv1a64"] == fnv1a64(data)
    assert result["inner_iterations"] == min(1024, max(1, (8 * 1024 * 1024 + size - 1) // size))
    assert len(result["trial_order"]) == 1
    assert set(result["trial_order"][0]) == METHODS
    assert len(result["trial_order"][0]) == len(METHODS)
    assert len(result["results"]) == len(METHODS)
    assert {row["method"] for row in result["results"]} == METHODS
    for row in result["results"]:
        assert set(row) == RESULT_FIELDS
        assert row["verified"] is True
        assert row["raw_pages"] + row["compressed_pages"] == pages
        metadata = 0 if row["method"] == "raw" else pages * 16
        assert row["metadata_bytes"] == metadata
        assert 0 <= row["payload_bytes"] <= size
        assert row["total_storage_bytes"] == row["payload_bytes"] + metadata
        if row["method"] == "raw":
            assert row["payload_bytes"] == size
            assert row["compressed_pages"] == 0
        if size == 1:
            assert row["payload_bytes"] == 1
            assert row["raw_pages"] == 1
            assert row["compressed_pages"] == 0
        for phase in ["encode", "decode"]:
            samples = row[f"{phase}_samples_ms"]
            assert len(samples) == 1
            assert math.isfinite(samples[0]) and samples[0] >= 0
            assert row[f"{phase}_ms"] == samples[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    rng = random.Random(20261030)
    records = bytearray()
    for i in range(4096):
        records.extend(struct.pack("<IIiI", 1700000000 + i, i % 37, (i % 500) - 250, i % 4))
    records.append(0xAB)
    fixtures = {
        "singleton": b"\x81",
        "zero_partial_page": bytes(65539),
        "random_partial_page": rng.randbytes(65537),
        "signed_records_partial_page": bytes(records),
    }
    with tempfile.TemporaryDirectory(prefix="pagecodec-smoke-") as directory:
        for name, content in fixtures.items():
            path = Path(directory) / (name + ".bin")
            path.write_bytes(content)
            run = subprocess.run(
                [str(binary), "--input", str(path), "--repetitions", "1", "--page-kib", "16", "--seed", "4129"],
                check=True, capture_output=True, text=True, timeout=90,
            )
            check_result(json.loads(run.stdout), content, path)
    print(f"PASS: {len(fixtures)} fixtures, {len(fixtures) * len(METHODS)} method cases; exact checksums and page accounting.")


if __name__ == "__main__":
    main()
