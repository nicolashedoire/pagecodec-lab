# Synthetic I/O journal research

This document records two bounded QEMU experiments, not an audit log for the Mac or the whole proposed OS. The first, [`io_journal`](../experiments/io_journal/README.md), tests a fixed metadata format using synthetic fixtures. The later [`journal_vm`](../experiments/journal_vm/README.md) integrates that format with selected synthetic inference requests in the graphical guest.

## Format and standalone test

The [`io_journal` codec](../experiments/io_journal/src/lib.rs) stores at most 16 entries in a canonical 512-byte sector. Each entry has a sequence number, category, actor label and outcome; it carries no payload. The decoder checks format magic, length, checksum, count, sequential numbering, field values and reserved bytes before returning a journal. When full, append rejects new records without overwriting existing ones. The FNV checksum detects incidental corruption, not malicious modification or authenticated identity.

The standalone [modern](../experiments/io_journal/evidence/io-journal-evidence.json) and [legacy](../experiments/io_journal/evidence/io-journal-legacy-evidence.json) QEMU traces cover save, readback, reconstruction and selected invalid fixtures. The 511-byte truncation case is a **logical input slice**, not a physically shortened disk. The test creates its synthetic disk exclusively in a private temporary directory; reconstruction inspects records rather than repeating recorded I/O.

## Inference events inside the graphical guest

[`journal_vm`](../experiments/journal_vm/src/main.rs) appends a metadata record after the `infer_synthetic` handler accepts or rejects arguments and before it emits its response. Its fixed `input` category means synthetic inference input, and `system` identifies the service, not an authenticated caller. It records **no input values, output scores, request bodies or private content**. It does not prove that a client received a response and does not record other MCP methods, malformed protocol lines, virtual-device operations or all machine I/O.

The 16-entry journal refuses further appends while inference still succeeds or rejects as usual. A saturating in-memory counter reports dropped events. Explicit `journal_save` writes sector 20, calls `flush()` and verifies readback; document snapshot slots use sectors 4–19. `journal_load` validates the sector before replacing in-memory entries. It replaces any unsaved journal and resets the dropped counter: **loss information is not persisted** in this format. Status, save and load calls are not logged themselves.

The [modern](../experiments/journal_vm/evidence/journal-vm-evidence.json) and [legacy](../experiments/journal_vm/evidence/journal-vm-legacy-evidence.json) evidence records accepted/rejected calls, saturation, continued inference, save, restart/load and rejection of a corrupt sector without replacing the active journal. The harness checks the sector's exact bytes and that unrelated disk bytes remain unchanged. Its disk is exclusively created in a private temporary directory. Additional [modern](../experiments/journal_vm/evidence/regression-evidence.json) and [legacy](../experiments/journal_vm/evidence/regression-legacy-evidence.json) traces exercise inherited graphical and snapshot behavior.

## Limits and next work

The journal is not comprehensive, tamper-resistant or an authenticated audit trail. A synthetically forged sector can carry forged labels. A successful driver `flush()` and readback do not establish that a FLUSH command was negotiated or that data survives power loss; interruption during the sector write and physical-device durability are not demonstrated. The integrated corruption test is a single checksum case, not exhaustive format or failed-device coverage.

A future broader journal needs an explicit event vocabulary, authenticated attribution, delivery and failure semantics, bounded retention with persistent loss reporting, and carefully defined privacy rules. It also needs tests for rejected writes, malformed but checksum-valid records, simultaneous producers, crash ordering and recovery. Those are design requirements, not properties established by the present QEMU fixtures.
