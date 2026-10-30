#![no_std]
#![no_main]
extern crate alloc;

use alloc::format;
use core::{alloc::{GlobalAlloc, Layout}, arch::{asm, global_asm}, panic::PanicInfo,
           ptr::addr_of_mut, sync::atomic::{AtomicUsize, Ordering}};
use actor_core_sketch::{ActorId, ActorKind, Position, SpaceId};
use dynamic_input_sketch::{Bounds, MoveError, Router, SetupError};

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
            if end > HEAP_SIZE { return core::ptr::null_mut(); }
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
fn outcome_name(outcome: Result<(), MoveError>) -> &'static str {
    match outcome { Ok(()) => "accepted", Err(MoveError::UnknownActor) => "unknown-actor",
        Err(MoveError::UnknownSpace) => "unknown-space", Err(MoveError::WrongSpace) => "wrong-space",
        Err(MoveError::OutOfBounds) => "out-of-bounds", Err(MoveError::Registry(_)) => "registry-error" }
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let bounds = Bounds { left: 0, top: 0, right: 100, bottom: 100 };
    let human = ActorId::new("human").unwrap();
    let ai = ActorId::new("assistant").unwrap();
    let remote = ActorId::new("remote").unwrap();
    let human_space = SpaceId::new("human-space").unwrap();
    let ai_space = SpaceId::new("assistant-space").unwrap();
    let remote_space = SpaceId::new("remote-space").unwrap();
    let mut router = Router::new();
    let invalid_space = SpaceId::new("invalid-space").unwrap();
    check(router.create_space(invalid_space.clone(), Bounds { left: 0, top: 0, right: 0, bottom: 10 }) == Err(SetupError::InvalidBounds), "invalid bounds setup");
    check(router.move_pointer(&human, &invalid_space, Position { x: 0, y: 0 }) == Err(MoveError::UnknownSpace), "invalid bounds changed registry");
    for space in [&human_space, &ai_space, &remote_space] { router.create_space(space.clone(), bounds).unwrap(); }
    for (actor, kind, space, position) in [(&human, ActorKind::Human, &human_space, Position { x: 10, y: 20 }),
                                          (&ai, ActorKind::Assistant, &ai_space, Position { x: 20, y: 30 }),
                                          (&remote, ActorKind::Remote, &remote_space, Position { x: 30, y: 40 })] {
        router.create_actor(actor.clone(), kind).unwrap();
        router.assign(actor, space).unwrap();
        check(router.move_pointer(actor, space, position).is_ok(), "valid move");
    }
    check(router.move_pointer(&ai, &human_space, Position { x: 5, y: 5 }) == Err(MoveError::WrongSpace), "inter-space rejection");
    check(router.move_pointer(&human, &human_space, Position { x: 100, y: 20 }) == Err(MoveError::OutOfBounds), "bounds rejection");
    let unknown_actor=ActorId::new("unknown").unwrap();
    let unknown_space=SpaceId::new("unknown-space").unwrap();
    check(router.move_pointer(&unknown_actor, &human_space, Position { x: 5, y: 5 }) == Err(MoveError::UnknownActor), "unknown actor");
    check(router.move_pointer(&human, &unknown_space, Position { x: 5, y: 5 }) == Err(MoveError::UnknownSpace), "unknown space");
    check(router.actor(&human).unwrap().pointer() == Some(Position { x: 10, y: 20 }), "human changed");
    check(router.actor(&ai).unwrap().pointer() == Some(Position { x: 20, y: 30 }), "assistant changed");
    check(router.actor(&remote).unwrap().pointer() == Some(Position { x: 30, y: 40 }), "remote changed");
    check(router.move_pointer(&human, &human_space, Position { x: 0, y: 0 }).is_ok(), "inclusive lower bounds");
    check(router.move_pointer(&human, &human_space, Position { x: 99, y: 99 }).is_ok(), "last valid point");
    check(router.move_pointer(&human, &human_space, Position { x: -1, y: 0 }) == Err(MoveError::OutOfBounds), "negative x");
    check(router.move_pointer(&human, &human_space, Position { x: 0, y: 100 }) == Err(MoveError::OutOfBounds), "exclusive y");
    check(router.actor(&human).unwrap().pointer() == Some(Position { x: 99, y: 99 }), "bounds failure changed state");
    for i in 0..16 {
        let actor = ActorId::new(&format!("agent-{i}")).unwrap();
        router.create_actor(actor.clone(), ActorKind::Remote).unwrap();
        router.assign(&actor, &remote_space).unwrap();
        check(router.move_pointer(&actor, &remote_space, Position { x: i, y: 90 }).is_ok(), "additional move");
    }
    check(router.actor_count() == 19, "dynamic actor count");
    check(router.actor(&remote).unwrap().pointer() == Some(Position { x: 30, y: 40 }), "shared-space pointer changed");
    check(router.traces().len() == 28, "trace count");
    // The bounded path must recycle its storage even with a non-reclaiming heap.
    let mut bounded=Router::new_bounded(64);
    let bounded_actor=ActorId::new("bounded").unwrap();
    let bounded_space=SpaceId::new("bounded-space").unwrap();
    let other_space=SpaceId::new("other-space").unwrap();
    bounded.create_space(bounded_space.clone(),bounds).unwrap();
    bounded.create_space(other_space.clone(),bounds).unwrap();
    bounded.create_actor(bounded_actor.clone(),ActorKind::Human).unwrap();
    bounded.assign(&bounded_actor,&bounded_space).unwrap();
    let heap_before=NEXT.load(Ordering::Relaxed);
    for i in 0..1100 { check(bounded.move_pointer_bounded(&bounded_actor,&bounded_space,Position{x:i%100,y:20}).is_ok(), "bounded repeated move"); }
    check(bounded.move_pointer_bounded(&bounded_actor,&bounded_space,Position{x:77,y:20}).is_ok(), "bounded last move");
    check(bounded.move_pointer_bounded(&bounded_actor,&other_space,Position{x:5,y:5})==Err(MoveError::WrongSpace), "bounded wrong space");
    check(bounded.actor(&bounded_actor).unwrap().pointer()==Some(Position{x:77,y:20}), "bounded reject changed pointer");
    check(bounded.bounded_trace().len()==64 && bounded.overwritten()==1038, "bounded retention");
    check(NEXT.load(Ordering::Relaxed)==heap_before, "bounded motion allocated");
    let last=bounded.bounded_trace().back().unwrap();
    check(last.outcome==Err(MoveError::WrongSpace) && last.actor_slot.and_then(|i|bounded.trace_actor(i))==Some(&bounded_actor) && last.space_slot.and_then(|i|bounded.trace_space(i))==Some(&other_space), "bounded trace slots");
    write("CHECK bounded_moves=1101 last=77,20 rejected_wrong_space=yes ring=64 overwritten=1038 motion_allocations=0\r\n");
    for record in router.traces() {
        // Trace comes from actual router records, not a separately fabricated log.
        write("IO actor="); write(record.actor.as_str()); write(" space="); write(record.space.as_str());
        write(" x="); write(&format!("{}", record.position.x)); write(" y="); write(&format!("{}", record.position.y));
        write(" result="); write(outcome_name(record.outcome)); write("\r\n");
    }
    write("CHECK actors=19 spaces=3 independent_pointers=yes rejected_attempts=logged\r\n");
    write("PASS synthetic dynamic input routing\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { write("FAIL kernel panic\r\n"); halt() }
