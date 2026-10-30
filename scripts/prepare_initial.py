#!/usr/bin/env python3
"""Recreate the 21 initial V2 inputs and verify their published SHA-256 hashes.

Requires Python 3.9+ and curl. Downloads are read as data; no downloaded code is
installed or executed. The expected manifest is never modified. The PNG fixture
is an already-compressed benchmark input, not an illustration of this project.
"""
import argparse
import csv
import hashlib
import io
import json
import random
import struct
import subprocess
import sys
import tarfile
from decimal import Decimal
from pathlib import Path


WEATHER_URL = (
    "https://raw.githubusercontent.com/vega/vega-datasets/"
    "5448e25297c8f705be78e879d5423d55d04dc6f1/data/seattle-weather.csv"
)
REA_URL = "https://registry.npmjs.org/rea-agents/-/rea-agents-6.8.0.tgz"
REA_MEMBER = "package/dist/cli-runtime/cli.js"
EXPECTED_NAMES = {
    "zeros.bin", "events_dev.bin", "events_holdout.bin", "events_shifted.bin",
    "random.bin", "scattered_palette.bin", "sparse_outliers.bin",
    "weather_dev.csv", "weather_holdout.csv", "weather_dev_i32.bin",
    "weather_holdout_i32.bin", "rea_cli_public.js", "previous_plot.png",
    "new_sparse_01pct.bin", "new_sparse_5pct.bin", "new_sparse_20pct.bin",
    "new_sparse_50pct.bin", "new_sparse_1pct_first_outlier.bin",
    "new_sparse_shifted_tail.bin", "new_alternating_sparse_dense.bin",
    "new_misleading_prefix.bin",
}


def download(url, destination, refresh=False):
    """Cache a fixed public source, writing only after a successful download."""
    if destination.is_file() and destination.stat().st_size and not refresh:
        return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".download")
    print(f"Downloading {url}", flush=True)
    try:
        subprocess.run(
            ["curl", "-fsSL", "--retry", "2", "--connect-timeout", "20",
             "--max-time", "180", url, "-o", str(temporary)], check=True
        )
        temporary.replace(destination)
    finally:
        if temporary.exists():
            temporary.unlink()
    return destination


def events(seed, noisy=False):
    """Preserve the original V1 generator, seed, layout, and RNG call order."""
    rng = random.Random(seed)
    output = bytearray(4 * 1024 * 1024)
    for i in range(len(output) // 16):
        timestamp = 1700000000 + i // 4
        device = rng.randrange(1000 if not noisy else 30000)
        temperature = rng.randrange(-250, 451)
        status = rng.randrange(8)
        if noisy and i % 2048 == 2047:
            temperature = rng.randrange(-(2**31), 2**31)
        struct.pack_into("<IIiI", output, i * 16,
                         timestamp, device, temperature, status)
    return output


def sparse_background(density, seed, first_outlier=False,
                      alternating=False, misleading=False):
    """Preserve the original V2 generator, including short-circuit RNG calls."""
    rng = random.Random(seed)
    size = 2 * 1024 * 1024
    background = 0x13579BDF
    output = bytearray(struct.pack("<I", background) * (size // 4))
    for i in range(size // 4):
        page = (i * 4) // 16384
        probability = (.01 if page % 2 == 0 else .50) if alternating else density
        force = ((first_outlier and i % 4096 in [0, 4095])
                 or (misleading and i % 4096 in [0, 1, 2]))
        if force or rng.random() < probability:
            value = rng.getrandbits(32)
            if value == background:
                value ^= 1
            struct.pack_into("<I", output, i * 4, value)
    return output


def prepare(manifest_path, output_dir, cache_dir, fixture_path, refresh):
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    datasets = manifest["datasets"]
    expected = {item["name"]: item for item in datasets}
    if len(expected) != len(datasets) or set(expected) != EXPECTED_NAMES:
        raise ValueError("The manifest must contain the exact 21 initial V2 dataset names.")
    output_dir.mkdir(parents=True, exist_ok=True)
    completed = set()

    def save(name, content):
        item = expected[name]
        actual_hash = hashlib.sha256(content).hexdigest()
        if len(content) != item["size"] or actual_hash != item["sha256"]:
            raise ValueError(
                f"Reproduction mismatch for {name}: {len(content)} bytes, "
                f"SHA-256 {actual_hash}; expected {item['size']} bytes, "
                f"SHA-256 {item['sha256']}. No mismatching output was written."
            )
        destination = output_dir / name
        temporary = destination.with_name(destination.name + ".building")
        temporary.write_bytes(content)
        temporary.replace(destination)
        completed.add(name)
        print(f"Verified {name}: {len(content)} bytes", flush=True)

    save("zeros.bin", bytes(1024 * 1024))
    save("events_dev.bin", events(128773))
    held_out = events(982451653, True)
    save("events_holdout.bin", held_out)
    save("events_shifted.bin", b"X" + held_out)
    rng = random.Random(32452843)
    save("random.bin", rng.randbytes(4 * 1024 * 1024))
    rng = random.Random(49979687)
    palette = rng.sample(range(256), 16)
    save("scattered_palette.bin", bytes(rng.choices(palette, k=4 * 1024 * 1024)))
    rng = random.Random(67867967)
    sparse = bytearray(4 * 1024 * 1024)
    for i in range(len(sparse) // 4):
        if rng.randrange(200) == 0:
            struct.pack_into("<I", sparse, i * 4, rng.getrandbits(32))
    save("sparse_outliers.bin", sparse)

    weather_file = download(WEATHER_URL, cache_dir / "seattle-weather-pinned.csv", refresh)
    weather = list(csv.DictReader(weather_file.read_text(encoding="utf-8").splitlines()))
    if not weather:
        raise ValueError("Downloaded weather corpus contains no records.")
    for split, years in [("dev", {"2012", "2013"}),
                         ("holdout", {"2014", "2015"})]:
        selected = [row for row in weather if row["date"][:4] in years]
        stream = io.StringIO(newline="")
        writer = csv.DictWriter(stream, fieldnames=list(weather[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(selected)
        save(f"weather_{split}.csv", stream.getvalue().encode("utf-8"))
        numbers = bytearray()
        for row in selected:
            values = [Decimal(row[key]) * 10 for key in
                      ["precipitation", "temp_max", "temp_min", "wind"]]
            if not all(value == int(value) for value in values):
                raise ValueError("Weather value is not exactly representable at scale 10.")
            numbers.extend(struct.pack("<iiii", *(int(value) for value in values)))
        save(f"weather_{split}_i32.bin", numbers)

    archive = download(REA_URL, cache_dir / "rea-agents-6.8.0.tgz", refresh)
    with tarfile.open(archive, mode="r:gz") as package:
        member = package.getmember(REA_MEMBER)
        if not member.isfile() or member.size != expected["rea_cli_public.js"]["size"]:
            raise ValueError("Unexpected REA corpus member type or size.")
        source = package.extractfile(member)
        if source is None:
            raise ValueError("Could not read REA corpus member.")
        with source:
            content = source.read(member.size + 1)
    save("rea_cli_public.js", content)
    save("previous_plot.png", fixture_path.read_bytes())

    for label, density, seed in [
        ("01pct", .001, 179424673), ("5pct", .05, 15485863),
        ("20pct", .20, 86028121), ("50pct", .50, 104395303),
    ]:
        save(f"new_sparse_{label}.bin", sparse_background(density, seed))
    save("new_sparse_1pct_first_outlier.bin",
         sparse_background(.01, 1301081, first_outlier=True))
    save("new_sparse_shifted_tail.bin",
         b"X" + sparse_background(.01, 1299709) + bytes(range(14)))
    save("new_alternating_sparse_dense.bin",
         sparse_background(.01, 22801763489, alternating=True))
    save("new_misleading_prefix.bin",
         sparse_background(.01, 32416190071, misleading=True))
    if completed != EXPECTED_NAMES:
        raise RuntimeError("Not all expected inputs were generated.")
    total = sum(item["size"] for item in datasets)
    print(f"All 21 inputs match the published manifest ({total} bytes, "
          f"{total / 2**20:.2f} MiB). The manifest was not changed.")


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path,
                        default=root / "manifests" / "initial-v2.json")
    parser.add_argument("--output", type=Path, default=root / "data" / "initial")
    parser.add_argument("--cache", type=Path, default=root / "data" / "downloads")
    parser.add_argument("--refresh-downloads", action="store_true",
                        help="Replace the two cached public downloads before verifying outputs.")
    args = parser.parse_args()
    if sys.version_info < (3, 9):
        parser.error("Python 3.9 or newer is required.")
    prepare(args.manifest, args.output, args.cache,
            root / "fixtures" / "previous_plot.png", args.refresh_downloads)


if __name__ == "__main__":
    main()
