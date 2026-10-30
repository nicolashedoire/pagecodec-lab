# System core sketch in Rust

A small **user-space** design exercise for the proposed operating system. It
is not a kernel, bootable image, window manager, security service, MCP server,
AI model, or operating system. It does not change any settings on the host.
It is written in Rust 2021 and has no third-party dependencies.

## What works

- Two independently editable in-memory workspaces derive from one base document.
- Each document is a map from field names to string values; absence means a
  field is not present, which is distinct from an empty string.
- A deterministic three-way merge compares base, human and assistant values.
  Non-conflicting changes carry `Base`, `Human`, `Assistant` or `Both` origin;
  a deletion is retained as an attributed tombstone in the result.
- Divergent changes, including delete versus edit, are preserved in `Conflict`
  objects containing all three values. Conflicted fields are not silently
  chosen, and `materialize` refuses to return a complete merged document until
  conflicts have been resolved.
- A demonstration prints the merged fields and an unresolved conflict.

Attribution is based on **net values compared to the base**, not a real
operation history. Editing a value and restoring it to the base is therefore
indistinguishable from not editing it. A `Both` label means the final values
match after both changed, not a semantic proof of independent authorship.
The merge is field-level: it does not merge paragraphs, binaries, files,
application-specific structures, transactions or external side effects.

`DecisionAdvisor` is a Rust trait describing a future conflict recommendation;
there is no model, Jev integration or implementation. `CapabilityBroker` is a
trait describing a future policy-check interface; it does **not** authenticate
callers, enforce permissions, protect the OS or grant real capabilities. Its
presence must not be interpreted as a security boundary. The owner envisions
AI-managed conflict resolution and traceable provenance, but neither has been
implemented beyond these placeholders and simplified origin labels.

## Try it

Inside a disposable virtual machine with a Rust toolchain and `cargo`, run from
this directory:

```sh
cargo test --offline
cargo run --offline
cargo fmt --check
```

Five tests cover independent edits, unchanged fields, identical changes,
deletions, divergent edits, delete-versus-edit, deterministic ordering and
absence versus an empty value. The tool operates exclusively on fictional
in-memory data. To handle real work, the system would need authenticated
identities, versioned persistence, concurrent access, format-specific merge,
policy enforcement, proper error handling and a genuinely integrated advisor.

The initial checks were run on the host before the project owner specified that
future tests should run in a VM. Subsequent project tests must use a virtual
machine; this user-space sketch is not itself a guest OS or a VM.
