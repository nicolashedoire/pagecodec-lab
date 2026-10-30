#![no_std]
#![no_main]
extern crate alloc;

use alloc::format;
use core::{alloc::{GlobalAlloc, Layout}, arch::{asm, global_asm}, panic::PanicInfo,
           ptr::addr_of_mut, sync::atomic::{AtomicUsize, Ordering}};
use actor_core_sketch::{ActorId, ActorKind, Error, Event, IdError, Position, Registry, SpaceId};

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

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    check(ActorId::new("") == Err(IdError::Empty), "empty id");
    check(ActorId::new("hello world") == Err(IdError::InvalidCharacter), "invalid id");
    let human = ActorId::new("human").unwrap();
    let assistant = ActorId::new("assistant").unwrap();
    let remote = ActorId::new("remote").unwrap();
    let human_space = SpaceId::new("human-space").unwrap();
    let ai_space = SpaceId::new("assistant-space").unwrap();
    let remote_space = SpaceId::new("remote-space").unwrap();
    let mut state = Registry::new();
    for space in [&human_space, &ai_space, &remote_space] { state.create_space(space.clone()).unwrap(); }
    for (actor, kind, space, x) in [(&human, ActorKind::Human, &human_space, 10),
                                  (&assistant, ActorKind::Assistant, &ai_space, 20),
                                  (&remote, ActorKind::Remote, &remote_space, 30)] {
        state.create_actor(actor.clone(), kind).unwrap();
        state.assign(actor, space).unwrap();
        state.move_pointer(actor, space, Position { x, y: 40 }).unwrap();
    }
    write("EVENT human pointer=10,40 space=human-space\r\nEVENT assistant pointer=20,40 space=assistant-space\r\nEVENT remote pointer=30,40 space=remote-space\r\n");
    let event_count = state.events().len();
    check(state.move_pointer(&assistant, &human_space, Position { x: 999, y: 999 }) == Err(Error::WrongSpace), "cross-space move");
    check(state.events().len() == event_count, "failed action logged");
    check(state.actor(&human).unwrap().pointer() == Some(Position { x: 10, y: 40 }), "human pointer changed");
    check(state.actor(&assistant).unwrap().pointer() == Some(Position { x: 20, y: 40 }), "assistant pointer changed");
    check(state.actor(&remote).unwrap().pointer() == Some(Position { x: 30, y: 40 }), "remote pointer changed");
    for (actor, space, x) in [(&human, &human_space, 10), (&assistant, &ai_space, 20), (&remote, &remote_space, 30)] {
        check(state.events().contains(&Event::PointerMoved { actor: actor.clone(), space: space.clone(), position: Position { x, y: 40 } }), "event attribution");
    }
    check(state.create_actor(human.clone(), ActorKind::Remote) == Err(Error::DuplicateActor), "duplicate actor");
    check(state.create_space(human_space.clone()) == Err(Error::DuplicateSpace), "duplicate space");
    let missing_actor = ActorId::new("missing-actor").unwrap();
    let missing_space = SpaceId::new("missing-space").unwrap();
    check(state.assign(&missing_actor, &human_space) == Err(Error::UnknownActor), "unknown actor");
    check(state.assign(&human, &missing_space) == Err(Error::UnknownSpace), "unknown space");
    check(state.events().len() == event_count, "failed events changed log");
    for i in 0..16 {
        let id = ActorId::new(&format!("additional-{i}")).unwrap();
        state.create_actor(id.clone(), ActorKind::Remote).unwrap();
        state.assign(&id, &remote_space).unwrap();
    }
    check(state.actor_count() == 19 && state.space_count() == 3, "dynamic actors");
    state.assign(&assistant, &remote_space).unwrap();
    check(state.actor(&assistant).unwrap().pointer().is_none(), "reassignment retained pointer");
    write("CHECK invalid_ids=ok cross_space=rejected independent_pointers=ok attribution=ok\r\n");
    write("CHECK duplicates=rejected unknowns=rejected failed_events=unchanged\r\n");
    write("CHECK actor_count=19 spaces=3 reassign_resets_pointer=yes\r\n");
    write("PASS actor core synthetic scenario\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { write("FAIL kernel panic\r\n"); halt() }
