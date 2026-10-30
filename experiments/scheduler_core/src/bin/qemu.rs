#![no_std]
#![no_main]
extern crate alloc;

use core::{alloc::{GlobalAlloc, Layout}, arch::{asm, global_asm}, panic::PanicInfo,
           ptr::addr_of_mut, sync::atomic::{AtomicUsize, Ordering}};
use scheduler_core_sketch::{Error, Scheduler, TaskId, TaskState};

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
    check(TaskId::new(0).is_none(), "zero id");
    let one = TaskId::new(1).unwrap();
    let two = TaskId::new(2).unwrap();
    let three = TaskId::new(3).unwrap();
    let unknown = TaskId::new(99).unwrap();
    let mut scheduler = Scheduler::new();
    check(scheduler.dispatch() == Ok(None), "empty queue");
    for id in [one, two, three] { check(scheduler.create(id).is_ok(), "create"); }
    check(scheduler.create(one) == Err(Error::DuplicateTask), "duplicate");
    check(scheduler.wake(unknown) == Err(Error::UnknownTask), "unknown wake");
    check(scheduler.wake(one) == Err(Error::NotBlocked), "wake ready");
    check(scheduler.block_running() == Err(Error::NoRunningTask), "block without running");
    check(scheduler.yield_running() == Err(Error::NoRunningTask), "yield without running");
    for id in [one, two, three, one, two, three] {
        check(scheduler.dispatch() == Ok(Some(id)), "round robin selection");
        check(scheduler.state(id) == Some(TaskState::Running), "running state");
        check(scheduler.dispatch() == Err(Error::AlreadyRunning), "second dispatch");
        check(scheduler.yield_running() == Ok(id), "yield");
        check(scheduler.state(id) == Some(TaskState::Ready), "ready state");
    }
    write("DECISIONS round_robin=1,2,3,1,2,3 voluntary_yield=yes\r\n");
    check(scheduler.dispatch() == Ok(Some(one)), "block selection");
    check(scheduler.block_running() == Ok(one) && scheduler.state(one) == Some(TaskState::Blocked), "block task");
    check(scheduler.dispatch() == Ok(Some(two)), "blocked skipped");
    check(scheduler.yield_running() == Ok(two), "yield task two");
    check(scheduler.wake(one).is_ok(), "wake blocked");
    check(scheduler.wake(one) == Err(Error::NotBlocked), "duplicate wake");
    check(scheduler.dispatch() == Ok(Some(three)), "wake appended to tail");
    check(scheduler.yield_running() == Ok(three), "yield task three");
    check(scheduler.dispatch() == Ok(Some(two)), "finish selection");
    check(scheduler.finish_running() == Ok(two) && scheduler.state(two) == Some(TaskState::Finished), "finish task");
    check(scheduler.wake(two) == Err(Error::NotBlocked), "wake finished");
    check(scheduler.create(two) == Err(Error::DuplicateTask), "reuse finished id");
    check(scheduler.dispatch() == Ok(Some(one)), "woken task selected");
    check(scheduler.finish_running() == Ok(one), "finish task one");
    check(scheduler.dispatch() == Ok(Some(three)), "remaining task selected");
    check(scheduler.finish_running() == Ok(three), "finish task three");
    check(scheduler.dispatch() == Ok(None) && scheduler.ready_count() == 0 && scheduler.running().is_none(), "finished empty");
    check(scheduler.finish_running() == Err(Error::NoRunningTask), "finish without running");
    write("DECISIONS blocked=1 skipped=1 wake_appended=1 finished=2,1,3 queue_empty=yes\r\n");
    write("CHECK invalid_transitions=rejected queue_integrity=ok task_ids=unique\r\n");
    write("PASS scheduler core synthetic scenario\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { write("FAIL kernel panic\r\n"); halt() }
