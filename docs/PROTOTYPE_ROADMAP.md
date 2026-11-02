# Integrated OS prototype: remaining work

Status at commit `300361b`: an experimental ARM64 guest boots in QEMU. It
runs up to three isolated, preempted ELF tasks on one virtual CPU, has bounded
IPC, virtual devices and two persisted synthetic document copies. Real guest
processes can access their assigned copy; the merge is deterministic. The
[CI run](https://github.com/nicolashedoire/pagecodec-lab/actions/runs/41362497874)
passed. See the [integrated guest](../experiments/workspace_dynamic_storage/README.md)
and its [current interfaces and limitations](../experiments/workspace_dynamic_storage/VFS.md).

This is a development order, not a claim that these milestones are complete.
Existing demonstrations and the intended graphical design are not a usable OS.

## First usable VM prototype

The milestone is a repeatable boot into a persistent graphical system where a
person and an AI work simultaneously in separate workspaces. Each has an
independent pointer, keyboard/focus and application state. A real document
can be changed in parallel, its revisions reconciled with attribution, saved,
and reopened after restart. A real AI service uses an authorized MCP control
interface; voice interaction is available through a floating control. The UI
follows the owner's mockups. A remote participant is an extension of the same
actor model; no fixed actor count is promised.

## Ordered work

1. **Memory and process foundations.** Replace the fixed three-slot task and
   frame limits with resource-managed allocation, private user heaps, process
   lifecycle and cleanup. Extend the restricted static ELF loader and runtime
   to support real applications and services. Handle resource exhaustion
   without halting the guest.
2. **Files and recovery.** Replace the bounded flat synthetic catalog with a
   practical filesystem and directories, larger files, a defined access API,
   quotas and a verified recovery strategy. Two-slot snapshots and readback do
   not prove power-loss durability. Bound the current in-memory event history.
3. **System services and authority.** Extend IPC and implement actor/session
   identity, permissions, revocable capabilities, resource limits and an
   authenticated MCP control plane. Current boot and binding IDs indicate
   freshness; they do not authenticate an actor or client.
4. **Application display and input.** Implement a compositor/window manager,
   independent window surfaces and focus, keyboard and pointing-event routing,
   lifecycle, and user-visible applications. Current panels and cursor demos
   do not provide independent graphical desktops.
5. **Concurrent workspaces and documents.** Connect real applications to
   separate actor workspaces and document revisions. Implement change
   attribution and recoverable merges, including contradictory edits. The
   existing two-copy field merge is not an AI conflict resolver or a general
   document format.
6. **AI and MCP integration.** Run a supported model and inference runtime,
   connect it to the actor workspace and scoped MCP tools, and implement and
   evaluate its merge decisions. Laya was tried separately on macOS; it is
   not integrated into the guest and is not Jev. Availability of local Jev
   weights has not been established here.
7. **Voice and interface.** Add audio input/output, a conversation pipeline
   (speech recognition and synthesis or direct speech-to-speech), and the
   floating voice widget; implement the supplied visual design. Dessa still
   needs a usable distribution, rights and runtime before integration can be
   established. It is a speech-to-speech model, not simply a TTS voice.
8. **Remote actor and scaling.** Add authenticated remote sessions with their
   own workspace and input, then measure actor, memory and communication
   limits. Existing isolated virtual Ethernet is not an IP stack or remote
   desktop. Flexible actor creation is the objective, not a promise of an
   arbitrary number of concurrent cursors.
9. **Whole-system reliability and performance.** Validate the combined user
   journey in the VM, including restart, error recovery, concurrent changes,
   resource exhaustion and authorization. Measure latency, memory and CPU on
   representative workloads; current one-CPU QEMU emulation and disabled
   caches do not demonstrate native Mac performance.

Milestones have dependencies but can be developed in parallel where interfaces
are stable. Rust remains the primary language; low-level assembly and necessary
interfaces may still be required.

## Physical Mac is a separate milestone

After a usable VM prototype, native M4 Pro boot, exact hardware description,
interrupts, DMA isolation and drivers for storage, display, input, network,
audio and power must be developed or suitably adapted and verified. Virtual
VirtIO devices do not prove support for Apple's physical controllers. See
[native boot research](NATIVE_BOOT_RESEARCH.md): no native installation
procedure has been established and no Mac installation is ready.
