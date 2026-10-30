# User-mode research: the first EL0 boundary

The [`user_mode` experiment](../experiments/user_mode/README.md) demonstrates a small, real AArch64 privilege transition inside a QEMU guest. It is a research component, not a process subsystem or a complete isolation mechanism. The code and recorded results are available for inspection: [guest implementation](../experiments/user_mode/src/main.rs), [linker layout](../experiments/user_mode/linker.ld), [test harness](../experiments/user_mode/run_demo.py), and [synthetic VM evidence](../experiments/user_mode/evidence/user-mode-evidence.json).

## What runs

The guest starts at EL1 on QEMU's virtual ARM machine, installs a bounded identity-mapped page table and a 2,048-byte-aligned exception vector table, then prepares `SP_EL0`, `ELR_EL1` and an EL0t `SPSR_EL1`. An `eret` enters a fixed synthetic function at EL0. The guest runs with interrupts masked and a single CPU.

The user function and its 4 KiB stack occupy separate pages. User code is mapped read-only and executable at EL0; the stack is writable and execute-never. Kernel data and the virtual UART are privileged mappings. Because the stack is in a `NOLOAD` section outside the startup BSS, the guest explicitly zeros and verifies all 4,096 bytes before exposing it to EL0. Selected address-translation queries and page descriptor checks validate the intended mappings; these checks are not an exhaustive audit of every address.

The EL0 function then performs this controlled sequence:

1. It writes a sentinel on its stack and attempts a load from a supplied kernel-data address. The processor raises a lower-EL data abort. The EL1 handler checks its exception class, permission-fault status, read direction, fault address and exact instruction address, then skips that single known load. The EL0 code checks that its destination-register sentinel was preserved.
2. It invokes `svc #0x1234`. The handler checks the syndrome, arguments, saved mode, stack and return address, changes the saved return value, and returns to EL0 with `eret`. The user function checks the response and selected register and stack sentinels.
3. It invokes `svc #0x5678`. This enters a non-returning EL1 finish routine that verifies the recorded events, the unchanged kernel datum and the user-stack sentinel.

Returning handler paths save and restore integer registers `x0`–`x30`; the scenario verifies only the selected values described above, not every register or condition flags. The data address passed to the test is synthetic, not a secret.

## Evidence and boundaries

The [recorded run](../experiments/user_mode/evidence/user-mode-evidence.json) reports successful completion under QEMU 11.1.3 and includes the guest image hash and serial trace. The harness checks the guest hash before and after execution. The tests exercise a virtual machine, not the physical Mac or Apple-specific hardware. The [experiment README](../experiments/user_mode/README.md) gives reproduction instructions and references the Arm exception and memory-management documentation.

This proves a particular EL0 read is denied and that the specified SVC round trip works in this controlled configuration. It does **not** establish complete security isolation: there is one fixed address space, no independent processes or address-space identifiers, no general syscall dispatcher, arbitrary program loader, scheduler integration, nested-exception recovery, FP/SIMD context management or DMA isolation. The final SVC does not return. Unexpected exceptions are fatal, and no latency or performance advantage is claimed.

## Scripted exchange between two address spaces

The separate [`user_ipc`](../experiments/user_ipc/README.md) guest uses two TTBR0 roots and ASIDs for a fixed sequence of eight SVCs. Two synthetic EL0 tasks exchange `HELLO` and `OK` via bounded kernel buffers and their own private page aliases. The handler checks the active actor/root, syndrome, saved mode, exact return site and phase, and checks the permitted page range before copying. The [QEMU trace](../experiments/user_ipc/evidence/user-ipc-evidence.json) includes rejection of a range crossing a page, an oversized length and the `u64::MAX - 1` plus 5 overflow case. In the latter, the range predicate's checked addition fails before a user-memory access; the scenario verifies the initially empty buffers remain unchanged.

These are cooperative switches and exact, prearranged request sites and values. The guest does not implement a general IPC API, arbitrary process scheduling, authenticated callers, dynamic mapping validation, general fault recovery or DMA isolation. It is a separate experiment from both `user_mode` and the graphical guest.

The separate [`ipc_service`](../experiments/ipc_service/README.md) guest extends this into a bounded dispatcher with a two-slot FIFO mailbox for each of the two fixed tasks. The kernel derives the fixture's task identity from its current context and verified TTBR, validates destination, size and checked user-page range, copies messages and returns status codes. The [QEMU trace](../experiments/ipc_service/evidence/ipc-service-evidence.json) covers cooperative switches, ordered two-way messages and selected full, empty, invalid destination, size and pointer rejections. It also checks that rejected operations leave mailboxes unchanged and that 8-bit diagnostic counters saturate at 255.

This identity belongs to fixed synthetic tasks; it does not authenticate a person or external MCP caller. The guest's immutable mappings and controlled copies are not a universal IPC interface: hardware faults during a copy are fatal, and atomicity across faults or concurrent changes is not established. General scheduling, dynamic page ownership and DMA isolation are not implemented. The separate [`workspace_ipc` diagnosis](../experiments/workspace_ipc/README.md) describes the broader connection to documents and MCP.

[`workspace_ipc_vm`](../experiments/workspace_ipc_vm/README.md) takes a narrower integration step: two fixed EL0 tasks each supply one eight-byte integer from their private page. The SVC verifies task and TTBR, copies from a fixed alias into one-shot per-task kernel state, and the EL1 service uses the numbers to derive synthetic document edits. The [modern](../experiments/workspace_ipc_vm/evidence/workspace-ipc-vm-modern.json) and [legacy](../experiments/workspace_ipc_vm/evidence/workspace-ipc-vm-legacy.json) QEMU traces check those copies, role/opcode/value rejections and read-only MCP observation. The slots are bitsets, not a general FIFO; the tasks do not consume their response codes. MCP is unauthenticated, no persistent workspace restoration is demonstrated, and the fixed task roles do not authenticate a person or establish generic IPC or DMA isolation.

The newer [`workspace_queue_vm`](../experiments/workspace_queue_vm/README.md) uses a two-entry kernel FIFO and three fixed requests with key/value bytes prefilled by EL1 in the tasks' private pages. The service applies these requests in order and associates completions with the fixed task and ID. A staged third submission encounters the full queue; task A checks `E_FULL=3` before draining begins. The tasks compare expected completion IDs and successful results before acknowledgment. [Modern](../experiments/workspace_queue_vm/evidence/workspace-queue-vm-modern.json) and [legacy](../experiments/workspace_queue_vm/evidence/workspace-queue-vm-legacy.json) evidence covers this scripted QEMU scenario. It does not establish general IPC, authenticated identities, persistent workspace restoration, recovery from arbitrary user-memory faults or DMA isolation.

## Remaining engineering work

- Define process and address-space ownership, protected mappings, lifecycle and zeroing rules, including reuse of physical pages and user stacks.
- Implement a bounded syscall ABI with argument validation and a deliberate policy for invalid calls and faults; test both success and failure paths.
- Extend exception and context handling to cover scheduler integration, interrupt delivery, register and flags preservation, and FP/SIMD state when enabled.
- Add independent address spaces and tests for cross-process reads and writes, kernel and device access, executable permissions, stack boundaries and page reuse. Test recovery rather than relying on one hard-coded skipped instruction.
- Integrate these mechanisms with the existing scheduler and memory work only after defining the contracts between them; the current standalone demonstrations do not yet form an integrated OS.
- Treat native Apple Silicon boot, device isolation and drivers as separate hardware work; a QEMU result alone does not establish those capabilities.
