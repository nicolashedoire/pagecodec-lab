# Native memory: what the reverse engineering established

We inspected the Mac's local ARM64e boot kernelcache and identified a real compressor-compaction decision in its machine code. This establishes a concrete mechanism to investigate. It does not establish a faster memory manager, a live kernel modification, or a performance improvement.

The sanitized observations and hashes are recorded in [native-memory-evidence.json](../manifests/native-memory-evidence.json). Proprietary binaries, Ghidra projects, raw disassembly, decompiler output, and machine-specific filesystem paths are not included in this repository.

## Evidence levels

| Evidence | What it establishes | What it does not establish |
| --- | --- | --- |
| Local native-binary inspection | Identity, load commands, embedded initial values, and the instructions present in the boot image | Current values in live kernel memory, execution frequency, or measured costs |
| Exact-version public XNU source | The documented implementation and downstream guards in Apple's `xnu-11215.41.3` release | Byte-for-byte identity of every component or configuration in Apple's proprietary ARM build |
| User-space experiment | Measured behavior of an explicitly implemented mechanism on the host | The performance of replacing XNU's mechanism, hardware compression, or system-wide scheduling |
| Simulation or policy replay | Decisions and modeled tradeoffs under stated assumptions | Measured physical-page reclamation or end-to-end machine acceleration |

This document reports the first two levels. It contains no compaction-benchmark result. These distinctions also apply to any subsequent experiment using this evidence.

## The native image was identified before analysis

The input was a locally obtained boot kernelcache, decoded to a 109,740,032-byte Mach-O fileset. Its SHA-256 is `8dab3ec8cfd458bf6704bee437e51edd498e3c392dca3cab9f55bdb08ab33e3f`.

[REA](https://github.com/morluto/rea) 6.8.0, using its `native-macos` inspection provider, reported little-endian 64-bit ARM64e, Mach-O file type 12 (`MH_FILESET`), 382 load commands, and UUID `D80B13B3-E724-0280-FC39-5016BE107E39`. Independent parsing found 369 `LC_FILESET_ENTRY` records. The `com.apple.kernel` entry starts at file offset `0x8000`, image virtual address `0xfffffe000700c000`.

Two embedded Darwin version strings exactly matched the identified running build: Darwin 24.1.0, `xnu-11215.41.3~13/RELEASE_ARM64_T6041`, build timestamp November 14, 2024, 18:15:21 PST. This avoids confusing an available Intel kernel image with the native Apple Silicon boot image. All addresses below are **image virtual addresses**, not observed live KASLR addresses.

## Ghidra recovered a bounded policy fragment

[Ghidra](https://github.com/NationalSecurityAgency/ghidra) 12.1.4 extracted `com.apple.kernel` through its own Mach-O fileset implementation, including chained-pointer fixups. Headless import selected `Extracted Mach-O File Set Entry` and language `AARCH64:LE:64:AppleSilicon`. Execution used Java 21, a 2 GiB Java heap, an active-processor limit of two, `-max-cpu 2`, and `-noanalysis`; the study did not run whole-program auto-analysis.

This extracted intermediate is specific to Ghidra's loader. REA's LLVM-backed inspection of it reported a section-offset error. Successful Ghidra import therefore must not be described as validation of a standalone Mach-O compatible with all native tools. REA successfully inspected the original fileset.

The targeted search produced the following chain of observations:

1. The embedded name `vm_compressor_minor_frag_threshold_pct` is at `0xfffffe000bf835c0`.
2. Its descriptor at `0xfffffe000bf835e8` points to an eight-byte variable at `0xfffffe000bf8e3d8`, whose **image initial value is 10**. The current runtime value was not read.
3. A targeted AArch64 ADRP/LDR search found a matching variable load at `0xfffffe00087c0bc4`. This is one match under that search pattern, not proof that no other addressing form can access the variable.
4. `LC_FUNCTION_STARTS` bounds the containing function to `[0xfffffe00087c0aec, 0xfffffe00087c0ce0)`: 500 bytes, or 125 fixed-width instructions.
5. Every byte in that extracted code range matches the corresponding range of the original cache. The range's SHA-256 is `21cf4476906f51f4342c56d1124260c29dcfb30a6193cb6c7faae6ad344a0b97`.

Within that bound, the assembly checks pending work and memory pressure, converts a page count to bytes with a 14-bit shift (16 KiB pages), computes unused bytes, loads the percentage variable, and compares fragmentation with the percentage of allocated pool bytes. A subsequent branch leads to locking, a flag update, and a wake-up call.

This strongly corresponds to public XNU's [`vm_consider_waking_compactor_swapper()`](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2964-L2998), with [`vm_compressor_needs_to_minor_compact()`](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L450-L473) inlined. The internal names are stripped. The attribution is therefore **source-assisted inference**; the instructions, variable initial value, and byte identity are direct observations. The 10% value is a global wake-up condition under additional guards, not a per-segment instruction to copy data whenever a hole reaches 10%.

## Decompiler success was not accepted as proof

Ghidra successfully produced bounded disassembly and pseudo-C for two symbol-identified exported interfaces: `mach_vm_pressure_monitor` and `mach_vm_pressure_level_monitor`. These expose pressure observation and waiting behavior; they are not replacement policies.

The candidate compaction function's pseudo-C was rejected. Even after explicitly bounding its Function body with `LC_FUNCTION_STARTS` and defining direct callees, the decompiler followed paths outside the body and emitted thousands of lines with PIC/branch warnings. Its `decompileCompleted=true` result was insufficient. The unreliable pseudo-C remains excluded from this repository and from the evidence supporting the policy claim. The accepted result is the bounded 125-instruction assembly, checked against the original bytes.

## Apple's downstream guards define the baseline

The following findings come from the **exact public source tag**, not a completed binary audit of every downstream function. They are necessary to avoid comparing an experiment against a weakened imitation of Apple's policy.

| Mechanism | Exact-source behavior | Experimental consequence |
| --- | --- | --- |
| Segment capacity | macOS ARM64 defaults to **64 KiB**; a boot argument can override it. Apple describes a performance/swap-write tradeoff for fast SSDs and hardware compression. | Use 64 KiB as the source-based default, not a measured live setting. [Definition](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L68-L95), [override](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L832-L843). |
| Minor-compaction admission | Skip when there is no empty slot or fewer than **one page** of unused bytes. Unused bytes include holes and populated trailing slack. | Do not give the baseline pointless sub-page copies. At 16 KiB pages, 10% of a 64 KiB segment is insufficient. [Guard](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2217-L2225), [accounting](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor_xnu.h#L291-L304). |
| Local scheduling | Queue eligible nonempty segments at one page of unused space; a segment already queued may be compacted immediately at one quarter of segment capacity. | Include local scheduling; the global 10% wake gate is not the only entry path. [Scheduling](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L5484-L5496), [quarter-segment threshold](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor_xnu.h#L304). |
| Bytes moved | Start at the **first hole** and move later live slots, preserving alignment and updating their mappings. | Charge the baseline only for the data it moves; copying every live record would exaggerate its cost. [Compaction loop](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2236-L2271). |
| Physical release | Round the compacted end to a page boundary and call `kernel_memory_depopulate` on the released tail. | Logical free bytes and physical pages returned are separate metrics. [Release](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2267-L2284). |
| Empty segments | Removing the last live slot can already release resident physical pages in the decompression/free path, before delayed structural cleanup. | Do not credit a new selector with a gain Apple already obtains on empty segments. [Empty-segment handling](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L5429-L5475). |
| Major compaction | Reject a pair when both segments are at least 90% full, or the destination has reached capacity/slot limits. The destination may need new physical pages before the donor releases pages. | Count net physical release, including destination growth. [Admission](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2359-L2381), [population](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2433-L2455), [donor cleanup](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L3140-L3159). |

Apple also avoids blocking in the opportunistic minor-compaction lock attempt, can abandon consolidation when a segment is wanted, and records moved bytes and bytes freed per microsecond. The inspected code treats the rate as instrumentation; it does not establish a learned prediction model selecting segments from that rate. [Nonblocking attempt](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L1508-L1531), [contention handling](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L3091-L3105), [yielding and rate](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L4078-L4115).

Another architecture-specific distinction matters: the historical thrashing detector's call path is explicitly disabled for macOS ARM64 in this tag. Its 250 ms evaluation window and age histogram must not be presented as the active policy on this machine without separate binary evidence. [Conditional exclusion](https://github.com/apple-oss-distributions/xnu/blob/xnu-11215.41.3/osfmk/vm/vm_compressor.c#L2735-L2755).

## Reproducing the local inspection

The original experiment workspace retains scripts and logs under `work/reverse-memory/ghidra/`. They and the proprietary inputs are **not distributed by this repository**, so these are instructions for repeating the local study, not a claim of a self-contained public reproduction bundle.

1. Obtain the native boot cache locally, decode its container, and check its architecture, Darwin string, digest, and `com.apple.kernel` fileset entry. Run REA 6.8.0's `inspect-macho` operation on the original cache. Different OS builds require new identification and addresses.
2. With Ghidra 12.1.4 and Java 21, run the local `scripts/ExtractKernel.java` as a headless pre-script, passing the decoded cache and the desired local output filename. This uses Ghidra's fileset extractor and fixups; do not replace it with a byte slice starting at `0x8000`.
3. Import the extracted entry with `-noanalysis -max-cpu 2`. Set `GHIDRA_HEADLESS_MAXMEM=2G` and `GHIDRA_HEADLESS_JAVA_OPTIONS=-XX:ActiveProcessorCount=2` for the recorded resource limits.
4. From the original workspace root, run `python3 work/reverse-memory/ghidra/locate_policy.py`. This reconstructs the tunable descriptor, scans the instruction pattern, and reads `LC_FUNCTION_STARTS` boundaries. It writes `minor-compaction-candidates.json` locally.
5. `scripts/InspectMemory.java` performs the initial targeted inspection. `scripts/BoundedPolicy.java` restricts the candidate's body to the recorded boundaries and exports its bounded assembly. The latter's candidate addresses are specific to this build. **Its generated candidate pseudo-C still fails the validity check and must be excluded.**
6. Compare the 500 original bytes at file offset 24,890,092 with the 500 extracted bytes at offset 3,793,116, then hash the range. Preserve the bounded assembly and logs locally; publish only sanitized observations, provenance, and hashes.

A subsequent user-space experiment should preserve the page guard, first-hole copying, metadata updates, empty-segment handling, and physical-release accounting. A guarded FIFO baseline and a simple cost-per-page selector should precede any learned selector. That experiment can test a memory-management mechanism; it cannot by itself demonstrate that modifying the native kernel makes the Mac faster.
