#!/usr/bin/env python3
"""Generate a byte-verified Apple Edge subset and run functional checks only.

No downloads, system changes, private SDK, live process inspection or timings.
All generated artifacts stay beside this script under generated/ and evidence/.
"""
from pathlib import Path
import hashlib
import json
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
UPSTREAM = ROOT / "upstream"
GENERATED = ROOT / "generated"
EVIDENCE = ROOT / "evidence"
BASE = "https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/"


def run(command, *, stdin=None):
    result = subprocess.run(command, input=stdin, text=True, capture_output=True, cwd=ROOT)
    return {"command": command, "returncode": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def git_blob_sha(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def main():
    GENERATED.mkdir(exist_ok=True)
    EVIDENCE.mkdir(exist_ok=True)
    manifest = json.loads((ROOT / "upstream-manifest.json").read_text())
    verification = []
    for entry in manifest["files"]:
        data = (UPSTREAM / entry["path"]).read_bytes()
        actual = git_blob_sha(data)
        if actual != entry["git_blob_sha1"]:
            raise RuntimeError("Upstream blob mismatch: " + entry["path"])
        verification.append({**entry, "verified_git_blob_sha1": actual,
                             "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})
    write_json(EVIDENCE / "upstream-verification.json", verification)

    specs = [
        ("apple_types.inc", "osfmk/kern/kern_types.h", 27,
         [(356, 365, "sched_clutch_edge"), (374, 380, "cluster_shared_rsrc_type_t")]),
        ("apple_metric.inc", "osfmk/kern/processor.h", 55,
         [(683, 698, "sched_get_pset_load_average")]),
        ("apple_edge.inc", "osfmk/kern/sched_clutch.c", 27,
         [(2241, 2249, "edge_shared_rsrc_policy"),
          (2303, 2313, "sched_edge_thread_shared_rsrc_type"),
          (3793, 3800, "sched_edge_pset_available"),
          (3829, 3834, "sched_edge_config_set"),
          (4025, 4032, "sched_edge_cluster_load_metric"),
          (4510, 4568, "sched_edge_migration_check"),
          (4579, 4610, "sched_edge_iterate_clusters_ordered"),
          (4620, 4681, "sched_edge_migrate_edges_evaluate"),
          (5203, 5246, "sched_edge_cpu_init_completed")]),
    ]
    blocks = []
    for output, source, license_lines, ranges in specs:
        lines = (UPSTREAM / source).read_bytes().splitlines(keepends=True)
        content = b"".join(lines[:license_lines])
        content += b"\n/* Generated verbatim source blocks; environment adapters live separately. */\n"
        for start, end, name in ranges:
            block = b"".join(lines[start-1:end])
            content += ('\n#line %d "upstream/%s"\n' % (start, source)).encode() + block
            blocks.append({"name": name, "source": source, "first_line": start,
                           "last_line": end, "bytes": len(block),
                           "sha256": hashlib.sha256(block).hexdigest(),
                           "url": BASE + source + "#L%d-L%d" % (start, end)})
        (GENERATED / output).write_bytes(content)
    write_json(EVIDENCE / "extracted-blocks.json", blocks)

    compiler = run(["xcrun", "clang", "--version"])
    sdk = run(["xcrun", "--show-sdk-path"])
    if compiler["returncode"] or sdk["returncode"]:
        raise RuntimeError("Public Command Line Tools unavailable")
    sdk_path = sdk["stdout"].strip()
    probes = []
    for header in ["darwintest.h", "darwintest_utils.h", "os/atomic_private.h", "stdatomic.h"]:
        probe = run(["xcrun", "clang", "-std=c11", "-isysroot", sdk_path,
                     "-fsyntax-only", "-x", "c", "-"], stdin="#include <%s>\n" % header)
        probe["header"] = header
        probes.append(probe)
    host = run(["sysctl", "kern.sched", "hw.ncpu", "hw.perflevel0.name",
                "hw.perflevel0.physicalcpu", "hw.perflevel1.name", "hw.perflevel1.physicalcpu"])
    write_json(EVIDENCE / "environment.json", {"compiler": compiler, "sdk": sdk,
        "header_probes": probes, "host_read_only": host,
        "performance_measurement": False,
        "note": "Header probes intentionally fail for unavailable internal dependencies."})

    common = ["xcrun", "clang", "-std=c11", "-O0", "-g", "-Wall", "-Wextra", "-Werror",
              "-Wno-sign-compare", "-isysroot", sdk_path, "edge_cases.c"]
    outputs = []
    for name, extra in [("edge-functional", []),
                        ("edge-functional-sanitized", ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"])]:
        binary = GENERATED / name
        compile_result = run(common + extra + ["-o", str(binary)])
        entry = {"variant": name, "compile": compile_result}
        if not compile_result["returncode"]:
            entry["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
            entry["execution"] = run([str(binary)])
        outputs.append(entry)
        write_json(EVIDENCE / "functional-results.json", outputs)
        if compile_result["returncode"] or entry.get("execution", {}).get("returncode"):
            print(json.dumps(entry, indent=2))
            return 1
    write_json(EVIDENCE / "local-file-hashes.json", {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in [ROOT / "edge_environment.h", ROOT / "edge_cases.c", Path(__file__),
                     *(GENERATED / s[0] for s in specs)]})
    print(json.dumps({"source_blobs_verified": len(verification),
                      "verbatim_blocks": len(blocks),
                      "variants": [{"name": x["variant"], "result": json.loads(x["execution"]["stdout"])}
                                   for x in outputs]}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
