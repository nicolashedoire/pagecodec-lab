#!/usr/bin/env python3
"""Recreate the verified Apple source cache from public URLs, without login.

Distribute this script, the manifest and the authored adapters/tests rather than
the upstream/ or generated/ directories. No source is accepted unless its Git
blob SHA-1 matches the checked-in manifest. Uses Python's standard library only.
"""
from pathlib import Path, PurePosixPath
import argparse
import hashlib
import json
import urllib.request

ROOT = Path(__file__).resolve().parent


def blob_sha(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-only", action="store_true", help="Never access the network or write files")
    args = parser.parse_args()
    manifest = json.loads((ROOT / "upstream-manifest.json").read_text())
    if manifest["repository"] != "apple-oss-distributions/xnu" or manifest["ref"] != "xnu-11215.41.3":
        raise ValueError("Unexpected upstream identity")
    downloaded = 0
    for item in manifest["files"]:
        path = PurePosixPath(item["path"])
        if path.is_absolute() or ".." in path.parts:
            raise ValueError("Unsafe source path")
        target = ROOT / "upstream" / path
        if target.exists():
            data = target.read_bytes()
        elif args.verify_only:
            raise FileNotFoundError(target)
        else:
            url = "https://raw.githubusercontent.com/apple-oss-distributions/xnu/xnu-11215.41.3/" + str(path)
            request = urllib.request.Request(url, headers={"User-Agent": "xnu-scheduler-public-source-fixture"})
            with urllib.request.urlopen(request, timeout=30) as response:
                data = response.read(2_000_001)
            if len(data) > 2_000_000:
                raise ValueError("Unexpectedly large source file")
            if blob_sha(data) != item["git_blob_sha1"]:
                raise ValueError("Downloaded source hash mismatch: " + str(path))
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            downloaded += 1
        if blob_sha(data) != item["git_blob_sha1"]:
            raise ValueError("Cached source hash mismatch: " + str(path))
    print(json.dumps({"source_files_verified": len(manifest["files"]), "downloaded": downloaded,
                      "verify_only": args.verify_only}))


if __name__ == "__main__":
    main()
