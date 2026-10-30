# Bounded SVC IPC dispatcher in QEMU

This experiment builds on [user_ipc](../user_ipc/README.md), replacing its phase-specific message handling with a small dispatcher keyed by the SVC immediate and the current EL0 actor. Two scripted EL0 contexts run in distinct TTBR0 address spaces on one QEMU AArch64 CPU. The same 4 KiB virtual address `0x50000000` resolves to separate physical pages; both pages are initially zeroed. Selected `AT S1E0R/W` queries check their mappings and privileged-only kernel/device addresses. This is not a test of an actual forbidden-address fault.

Each actor has a two-slot FIFO inbox in EL1, with 1–16 byte payloads. The user frame carries arguments; the dispatcher identifies the caller via a kernel-maintained current-actor variable and verifies the active TTBR, SVC exception class, EL0 state and aligned return address inside the fixed user-code region. It does **not** authorize requests by the exact SVC instruction address. These IDs are trusted fixtures, not authenticated identities.

| SVC immediate | Arguments | Result |
| --- | --- | --- |
| `0x10` send | `x0` source, `x1` length, `x2` destination actor | `x0` status |
| `0x11` receive | `x0` destination, `x1` capacity | `x0` status; `x1` bytes on success, zero on failure |
| `0x12` yield | none | `x0 = 0`, then switch to the other fixed context |
| `0x13` finish | actor A only | enter the fixture's EL1 final checks |

Status is `0` for success, `1` for an invalid range, `2` for invalid size or insufficient receive capacity, `3` for a full inbox, `4` for an empty inbox, and `5` for an invalid destination. A malformed exception or unsupported service stops the fixture instead of returning a general error. The copy uses byte accesses under the caller's active TTBR after checking the fixed alias range, `checked_add`, and size. Before any error response, the handler compares the complete mailbox state with a snapshot and fails if it changed. No user pointer is touched on an error. Immutable page tables, one CPU and masked interrupts make these fixture checks meaningful, but they do not provide a general dynamic-mapping or concurrent-copy design.

A sends `HELLO` and `PING` to B, which receives and checks both in FIFO order; B sends `OK` and `GO` to A, which checks both. The two contexts **voluntarily yield** between batches; they do not run concurrently. The script tests pointer wraparound, a range crossing the page, a kernel address, a 17-byte message, an invalid actor, a too-small receive buffer, full and empty inboxes. The initial scenario is followed by four generated rounds. In each round A fills B’s inbox with two one-byte messages and runs an independent EL0 arithmetic oracle against 9 source addresses from page-end minus 4 through page-end plus 4, each with length 4; only the first fits and therefore reports full, while the others report invalid range. It also tests 19 lengths from 0 through 18 at a valid start address: lengths 1–16 report full and 0, 17 and 18 report invalid size. B drains the two messages in order, checks their round-specific bytes and verifies empty; A also checks its empty inbox. Thus 112 generated rejection calls are checked against an oracle that does not call the kernel range helper, alongside four FIFO wraparound rounds. The dispatcher compares both complete inboxes against the pre-call snapshot on every error. End checks require the expected counts, empty mailboxes and exact contents of all bytes in both physical pages. The serial journal is emitted from this verified final state; it is not a general runtime audit stream.

The generated set covers specific boundaries and states; it is neither exhaustive nor a formal fuzzer. This is a bounded **experimental dispatcher, not a universal IPC service**: the address spaces, actors, yielding schedule, finish operation, stack pointer, page size and mailbox capacity are fixed. It has no authenticated identities, dynamic page ownership, general fault recovery, blocking/wakeup, cancellation, persistence, scheduling fairness or certified isolation. The fixture uses saturating 8-bit diagnostic counters, so they stop counting at 255; the guest exercises the saturation helper at its boundary. A hardware fault during a validated copy is fatal and is not recovered. No atomicity guarantee is made across faults or concurrent changes. It does not establish DMA/IOMMU isolation, SMP behavior, FP/SIMD preservation, Apple hardware support, or safe operation with real data. Full local TLB invalidation occurs on each root switch; this is not an ASID reuse or multicore shootdown design. Kernel UART output contains only synthetic markers.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3 installed, from this directory:

```sh
cargo build --release --locked --offline
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/ipc-service-demo \
  --output-dir /path/to/output
```

The runner launches a single TCG Cortex-A57 guest, with no network, disks or host-directory sharing. It checks the exact synthetic serial output, the guest hash before and after execution, and records the QEMU version. Do not rebuild during the run. See the [recorded evidence](evidence/ipc-service-evidence.json).
