#![no_std]
#![no_main]
extern crate alloc;

use core::{alloc::{GlobalAlloc, Layout}, arch::{asm, global_asm}, panic::PanicInfo,
           ptr::addr_of_mut, sync::atomic::{AtomicUsize, Ordering}};
use actor_core_sketch::{ActorId, ActorKind, Registry, SpaceId};
use system_core_sketch::{Document, Origin};
use workspace_bridge_sketch::{Bridge, Error};

global_asm!(r#"
    .section .text.boot, "ax"
    .global _start
_start:
    msr daifset, #0xf
    mrs x0, CurrentEL
    cmp x0, #4
    b.ne 3f
    mrs x0, SCTLR_EL1
    tbnz x0, #0, 3f
    tbnz x0, #2, 3f
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:  cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:  ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
3:  wfe
    b 3b
"#);

const HEAP_SIZE: usize = 128 * 1024;
#[repr(align(4096))]
struct Heap([u8; HEAP_SIZE]);
static mut HEAP: Heap = Heap([0; HEAP_SIZE]);
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Bump;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = addr_of_mut!(HEAP) as *mut u8 as usize;
        let mut current = NEXT.load(Ordering::Relaxed);
        loop {
            let Some(start) = base.checked_add(current).and_then(|n| n.checked_add(layout.align() - 1))
                .map(|n| (n & !(layout.align() - 1)) - base) else { return core::ptr::null_mut() };
            let Some(end) = start.checked_add(layout.size()) else { return core::ptr::null_mut() };
            if end > HEAP_SIZE { return core::ptr::null_mut() }
            match NEXT.compare_exchange_weak(current, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return (addr_of_mut!(HEAP) as *mut u8).add(start),
                Err(next) => current = next,
            }
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator]
static ALLOC: Bump = Bump;

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, message: &str) { if !ok { write("FAIL "); write(message); write("\r\n"); halt() } }

fn fixture() -> (Bridge, ActorId, SpaceId, ActorId, SpaceId, ActorId, SpaceId) {
    let human = ActorId::new("human").unwrap();
    let ai = ActorId::new("assistant").unwrap();
    let remote = ActorId::new("remote").unwrap();
    let hs = SpaceId::new("human-space").unwrap();
    let ais = SpaceId::new("assistant-space").unwrap();
    let rs = SpaceId::new("remote-space").unwrap();
    let mut registry = Registry::new();
    for space in [&hs, &ais, &rs] { registry.create_space(space.clone()).unwrap(); }
    for (id, kind, space) in [(&human, ActorKind::Human, &hs), (&ai, ActorKind::Assistant, &ais), (&remote, ActorKind::Remote, &rs)] {
        registry.create_actor(id.clone(), kind).unwrap();
        registry.assign(id, space).unwrap();
    }
    let mut base = Document::new();
    base.insert("title".into(), "base".into());
    let bridge = Bridge::new(registry, human.clone(), hs.clone(), ai.clone(), ais.clone(), base).unwrap();
    (bridge, human, hs, ai, ais, remote, rs)
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let (mut bridge, human, hs, ai, ais, remote, rs) = fixture();
    check(matches!(Bridge::new(Registry::new(), human.clone(), hs.clone(), human.clone(), ais.clone(), Document::new()), Err(Error::SameActor)), "same actor binding");
    check(matches!(Bridge::new(Registry::new(), human.clone(), hs.clone(), ai.clone(), hs.clone(), Document::new()), Err(Error::SameSpace)), "same space binding");
    let mut invalid_registry = Registry::new();
    invalid_registry.create_space(hs.clone()).unwrap();
    invalid_registry.create_actor(human.clone(), ActorKind::Remote).unwrap();
    invalid_registry.assign(&human, &hs).unwrap();
    check(matches!(Bridge::new(invalid_registry, human.clone(), hs.clone(), ai.clone(), ais.clone(), Document::new()), Err(Error::WrongKind)), "wrong kind binding");
    write("REQUEST actor=human space=human-space put title=human\r\n");
    bridge.edit(&human, &hs, "title", Some("human")).unwrap();
    write("RESPONSE accepted\r\nREQUEST actor=assistant space=assistant-space put status=assistant\r\n");
    bridge.edit(&ai, &ais, "status", Some("assistant")).unwrap();
    write("RESPONSE accepted\r\n");
    check(bridge.view(&human, &hs).unwrap().get("status").is_none(), "human copy polluted");
    check(bridge.view(&ai, &ais).unwrap().get("title").map(|s| s.as_str()) == Some("base"), "assistant copy polluted");
    let events_before = bridge.events().len();
    check(bridge.events()[0].actor == human && bridge.events()[0].space == hs && bridge.events()[0].key == "title", "human event attribution");
    check(bridge.events()[1].actor == ai && bridge.events()[1].space == ais && bridge.events()[1].key == "status", "assistant event attribution");
    check(bridge.edit(&ai, &hs, "title", Some("intrusion")) == Err(Error::WrongSpace), "wrong space");
    check(bridge.edit(&remote, &rs, "title", Some("remote")) == Err(Error::UnboundActor), "unbound remote");
    check(bridge.events().len() == events_before, "rejected event recorded");
    write("REQUEST assistant human-space edit; RESPONSE wrong-space\r\nREQUEST remote remote-space edit; RESPONSE unbound-actor\r\n");
    let merged = bridge.merge();
    check(merged == bridge.merge(), "merge not deterministic");
    check(merged.conflicts.is_empty(), "unexpected conflict");
    let title = merged.fields.get("title").unwrap();
    let status = merged.fields.get("status").unwrap();
    check(title.origin == Origin::Human && title.value.as_deref() == Some("human"), "human provenance");
    check(status.origin == Origin::Assistant && status.value.as_deref() == Some("assistant"), "assistant provenance");
    check(bridge.provenance(title.origin).0 == Some(&human), "human id provenance");
    check(bridge.provenance(status.origin).1 == Some(&ai), "assistant id provenance");
    check(merged.materialize().is_ok(), "compatible materialize");
    write("RESPONSE merge title=human origin=human status=assistant origin=assistant\r\n");

    let (mut conflict, human, hs, ai, ais, _, _) = fixture();
    conflict.edit(&human, &hs, "title", Some("human-version")).unwrap();
    conflict.edit(&ai, &ais, "title", Some("assistant-version")).unwrap();
    let result = conflict.merge();
    check(result.conflicts.len() == 1 && result.materialize().is_err(), "conflict discarded");
    let c = &result.conflicts[0];
    check(c.key == "title" && c.base.as_deref() == Some("base") && c.human.as_deref() == Some("human-version") && c.assistant.as_deref() == Some("assistant-version"), "conflict values");
    write("REQUEST both edit title differently\r\nRESPONSE conflict base=base human=human-version assistant=assistant-version materialize=rejected\r\n");

    let (mut deletion, human, hs, ai, ais, _, _) = fixture();
    deletion.edit(&human, &hs, "title", None).unwrap();
    deletion.edit(&ai, &ais, "status", Some("ready")).unwrap();
    let result = deletion.merge();
    check(result.fields.get("title").is_some_and(|f| f.value.is_none() && f.origin == Origin::Human), "delete provenance");
    check(!result.materialize().unwrap().contains_key("title"), "delete materialize");
    let (mut both, human, hs, ai, ais, _, _) = fixture();
    both.edit(&human, &hs, "title", Some("same")).unwrap();
    both.edit(&ai, &ais, "title", Some("same")).unwrap();
    check(both.merge().fields.get("title").is_some_and(|f| f.origin == Origin::Both), "both provenance");
    check(both.provenance(Origin::Both) == (Some(&human), Some(&ai)), "both actor IDs");
    write("CHECK bindings=validated delete_tombstone=human same_change=both deterministic=yes\r\n");
    write("PASS workspace bridge synthetic scenario\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { write("FAIL kernel panic\r\n"); halt() }
