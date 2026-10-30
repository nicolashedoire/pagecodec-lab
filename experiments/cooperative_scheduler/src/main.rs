#![no_std]
#![no_main]

use core::{alloc::{GlobalAlloc, Layout}, arch::{asm, global_asm}, panic::PanicInfo, ptr::{addr_of, addr_of_mut, read_volatile}, sync::atomic::{AtomicUsize, Ordering}};
use scheduler_core_sketch::{Scheduler, TaskId, TaskState};

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
    msr SPSel, #1
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

    .section .text.switch, "ax"
    // x0 = old Context, x1 = new Context. Layout: SP, x19..x30.
    .global context_switch
context_switch:
    mov x2, sp
    str x2, [x0, #0]
    stp x19, x20, [x0, #8]
    stp x21, x22, [x0, #24]
    stp x23, x24, [x0, #40]
    stp x25, x26, [x0, #56]
    stp x27, x28, [x0, #72]
    stp x29, x30, [x0, #88]
    ldp x19, x20, [x1, #8]
    ldp x21, x22, [x1, #24]
    ldp x23, x24, [x1, #40]
    ldp x25, x26, [x1, #56]
    ldp x27, x28, [x1, #72]
    ldp x29, x30, [x1, #88]
    ldr x2, [x1, #0]
    mov sp, x2
    ret

    // x0 old, x1 new, x2 seed. Check x19..x29 and SP survive suspension.
    .global yield_probe
yield_probe:
    sub sp, sp, #128
    stp x19, x20, [sp, #0]
    stp x21, x22, [sp, #16]
    stp x23, x24, [sp, #32]
    stp x25, x26, [sp, #48]
    stp x27, x28, [sp, #64]
    stp x29, x30, [sp, #80]
    str x2, [sp, #96]
    mov x3, sp
    str x3, [sp, #104]
    add x19, x2, #19
    add x20, x2, #20
    add x21, x2, #21
    add x22, x2, #22
    add x23, x2, #23
    add x24, x2, #24
    add x25, x2, #25
    add x26, x2, #26
    add x27, x2, #27
    add x28, x2, #28
    add x29, x2, #29
    bl context_switch
    ldr x1, [sp, #96]
    add x0, x1, #19
    cmp x19, x0
    b.ne probe_fail
    add x0, x1, #20
    cmp x20, x0
    b.ne probe_fail
    add x0, x1, #21
    cmp x21, x0
    b.ne probe_fail
    add x0, x1, #22
    cmp x22, x0
    b.ne probe_fail
    add x0, x1, #23
    cmp x23, x0
    b.ne probe_fail
    add x0, x1, #24
    cmp x24, x0
    b.ne probe_fail
    add x0, x1, #25
    cmp x25, x0
    b.ne probe_fail
    add x0, x1, #26
    cmp x26, x0
    b.ne probe_fail
    add x0, x1, #27
    cmp x27, x0
    b.ne probe_fail
    add x0, x1, #28
    cmp x28, x0
    b.ne probe_fail
    add x0, x1, #29
    cmp x29, x0
    b.ne probe_fail
    mov x0, sp
    ldr x1, [sp, #104]
    cmp x0, x1
    b.ne probe_fail
    mov x0, #1
    b probe_restore
probe_fail:
    mov x0, #0
probe_restore:
    ldp x19, x20, [sp, #0]
    ldp x21, x22, [sp, #16]
    ldp x23, x24, [sp, #32]
    ldp x25, x26, [sp, #48]
    ldp x27, x28, [sp, #64]
    ldp x29, x30, [sp, #80]
    add sp, sp, #128
    ret
"#);

const STACK_BYTES: usize = 16 * 1024;
#[repr(C, align(16))]
struct Stack([u8; STACK_BYTES]);
#[repr(C)]
struct Context { words: [u64; 13] }
static mut MAIN_CONTEXT: Context = Context { words: [0; 13] };
static mut A_CONTEXT: Context = Context { words: [0; 13] };
static mut B_CONTEXT: Context = Context { words: [0; 13] };
static mut A_STACK: Stack = Stack([0; STACK_BYTES]);
static mut B_STACK: Stack = Stack([0; STACK_BYTES]);
static mut STEP: u64 = 0;
const HEAP_BYTES: usize = 128 * 1024;
#[repr(align(4096))]
struct Heap([u8; HEAP_BYTES]);
static mut HEAP: Heap = Heap([0; HEAP_BYTES]);
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
            if end > HEAP_BYTES { return core::ptr::null_mut() }
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

const ACTION_YIELD: u64 = 1;
const ACTION_BLOCK: u64 = 2;
const ACTION_FINISH: u64 = 3;
static mut ACTION: u64 = 0;

extern "C" {
    fn yield_probe(old: *mut Context, new: *const Context, seed: u64) -> u64;
    static __bss_end: u8;
    static __stack_top: u8;
}
fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, what: &str) { if !ok { write("FAIL "); write(what); write("\r\n"); halt(); } }
fn tick(expected: u64) {
    unsafe {
        let current = read_volatile(addr_of!(STEP));
        check(current == expected, "unexpected order");
        addr_of_mut!(STEP).write_volatile(current + 1);
    }
}
fn current_sp() -> usize { let sp: usize; unsafe { asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags)); } sp }
// Treat linker boundaries and static addresses as observed physical addresses.
fn physical_address(value: usize) -> usize {
    let observed: usize;
    unsafe { asm!("mov {observed}, {value}", observed = out(reg) observed,
                  value = in(reg) value, options(nomem, nostack, preserves_flags)); }
    observed
}
fn write_hex(value: usize) {
    const HEX: &[u8;16] = b"0123456789abcdef";
    write("0x");
    for shift in (0..16).rev() {
        let b = HEX[(value >> (shift * 4)) & 15];
        unsafe { while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
                 (0x0900_0000 as *mut u32).write_volatile(b as u32); }
    }
}
fn stack_position(start: usize, top: usize, sp: usize) -> bool {
    start < top && sp > start && sp <= top && sp % 16 == 0
}
fn stack_contains(stack: *const Stack, sp: usize) -> bool { let start = stack as usize; sp >= start && sp < start + STACK_BYTES && sp % 16 == 0 }
fn object_on_stack(stack: *const Stack, address: usize, length: usize) -> bool {
    let start = stack as usize;
    address >= start && address.checked_add(length).is_some_and(|end| end <= start + STACK_BYTES)
}

fn set_action(value: u64) { unsafe { addr_of_mut!(ACTION).write_volatile(value); } }
fn sentinel_ok(p: *const u64, a: u64, b: u64) -> bool {
    unsafe { read_volatile(p) == a && read_volatile(p.add(1)) == b }
}

#[no_mangle]
extern "C" fn task_a() -> ! {
    let mut sentinel = [0u64; 2];
    let p = sentinel.as_mut_ptr();
    check(object_on_stack(addr_of!(A_STACK), p as usize, 16), "A sentinel stack");
    unsafe { p.write_volatile(0x1122_3344_5566_7788); p.add(1).write_volatile(0x0102_0304_0506_0708); }
    check(stack_contains(addr_of!(A_STACK), current_sp()), "A stack");
    tick(0); write("TASK a stage=1 action=yield\r\n"); set_action(ACTION_YIELD);
    check(unsafe { yield_probe(addr_of_mut!(A_CONTEXT), addr_of!(MAIN_CONTEXT), 0x100) } == 1, "A registers after first yield");
    check(sentinel_ok(p, 0x1122_3344_5566_7788, 0x0102_0304_0506_0708), "A stack after first yield");
    tick(2); write("TASK a stage=2 action=yield\r\n"); set_action(ACTION_YIELD);
    check(unsafe { yield_probe(addr_of_mut!(A_CONTEXT), addr_of!(MAIN_CONTEXT), 0x200) } == 1, "A registers after second yield");
    check(sentinel_ok(p, 0x1122_3344_5566_7788, 0x0102_0304_0506_0708), "A stack after second yield");
    tick(3); write("TASK a stage=3 action=finish\r\n"); set_action(ACTION_FINISH);
    unsafe { yield_probe(addr_of_mut!(A_CONTEXT), addr_of!(MAIN_CONTEXT), 0x300); }
    write("FAIL finished A resumed\r\n"); halt();
}

#[no_mangle]
extern "C" fn task_b() -> ! {
    let mut sentinel = [0u64; 2];
    let p = sentinel.as_mut_ptr();
    check(object_on_stack(addr_of!(B_STACK), p as usize, 16), "B sentinel stack");
    unsafe { p.write_volatile(0x8877_6655_4433_2211); p.add(1).write_volatile(0x0807_0605_0403_0201); }
    check(stack_contains(addr_of!(B_STACK), current_sp()), "B stack");
    tick(1); write("TASK b stage=1 action=block\r\n"); set_action(ACTION_BLOCK);
    check(unsafe { yield_probe(addr_of_mut!(B_CONTEXT), addr_of!(MAIN_CONTEXT), 0x400) } == 1, "B registers after wake");
    check(sentinel_ok(p, 0x8877_6655_4433_2211, 0x0807_0605_0403_0201), "B stack after wake");
    tick(4); write("TASK b stage=2 action=finish\r\n"); set_action(ACTION_FINISH);
    unsafe { yield_probe(addr_of_mut!(B_CONTEXT), addr_of!(MAIN_CONTEXT), 0x500); }
    write("FAIL finished B resumed\r\n"); halt();
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let a = physical_address(addr_of!(A_STACK) as usize);
    let b = physical_address(addr_of!(B_STACK) as usize);
    let main_start = physical_address(addr_of!(__bss_end) as usize);
    let main_top = physical_address(addr_of!(__stack_top) as usize);
    let main_sp = current_sp();
    let valid = match (a.checked_add(STACK_BYTES), b.checked_add(STACK_BYTES)) {
        (Some(a_end), Some(b_end)) => a % 16 == 0 && b % 16 == 0 && main_start % 16 == 0 && main_top % 16 == 0
            && (a_end <= b || b_end <= a)
            && (a_end <= main_start || main_top <= a) && (b_end <= main_start || main_top <= b)
            && stack_position(main_start, main_top, main_sp),
        _ => false,
    };
    if !valid {
        write("STACK_DIAGNOSTIC a="); write_hex(a); write(" b="); write_hex(b);
        write(" main_start="); write_hex(main_start); write(" main_top="); write_hex(main_top);
        write(" main_sp="); write_hex(main_sp); write("\r\n");
    }
    check(valid, "disjoint aligned stacks");
    check(stack_position(main_start, main_top, main_top), "empty main stack boundary");
    check(!stack_position(main_start, main_top, main_start), "main stack bottom boundary");
    check(main_top.checked_add(16).is_some_and(|outside| !stack_position(main_start, main_top, outside)), "outside main stack boundary");
    write("CHECK stack_boundaries=top_allowed_bottom_outside_rejected opaque_linker_addresses=yes\r\n");
    unsafe {
        (*addr_of_mut!(A_CONTEXT)).words[0] = (a + STACK_BYTES) as u64;
        (*addr_of_mut!(A_CONTEXT)).words[12] = task_a as *const () as usize as u64;
        (*addr_of_mut!(B_CONTEXT)).words[0] = (b + STACK_BYTES) as u64;
        (*addr_of_mut!(B_CONTEXT)).words[12] = task_b as *const () as usize as u64;
    }
    let a_id = TaskId::new(1).unwrap();
    let b_id = TaskId::new(2).unwrap();
    let mut scheduler = Scheduler::new();
    check(scheduler.create(a_id).is_ok() && scheduler.create(b_id).is_ok(), "create tasks");
    for (i, expected) in [a_id, b_id, a_id, a_id, b_id].into_iter().enumerate() {
        let Some(selected) = scheduler.dispatch().unwrap() else { write("FAIL unexpected empty queue\r\n"); halt(); };
        check(selected == expected, "scheduler order");
        let context = if selected == a_id {
            write("SCHED dispatch=a\r\n"); addr_of!(A_CONTEXT)
        } else if selected == b_id {
            write("SCHED dispatch=b\r\n"); addr_of!(B_CONTEXT)
        } else {
            write("FAIL no context for selected task\r\n"); halt();
        };
        set_action(0);
        check(unsafe { yield_probe(addr_of_mut!(MAIN_CONTEXT), context, 0x600 + i as u64) } == 1, "main registers or SP");
        let action = unsafe { read_volatile(addr_of!(ACTION)) };
        match action {
            ACTION_YIELD => check(scheduler.yield_running() == Ok(selected), "scheduler yield"),
            ACTION_BLOCK => check(scheduler.block_running() == Ok(selected), "scheduler block"),
            ACTION_FINISH => check(scheduler.finish_running() == Ok(selected), "scheduler finish"),
            _ => { write("FAIL missing task action\r\n"); halt(); }
        }
        if i == 1 { check(scheduler.state(b_id) == Some(TaskState::Blocked), "B not blocked"); write("SCHED b=blocked\r\n"); }
        if i == 2 {
            check(scheduler.state(b_id) == Some(TaskState::Blocked), "B woke early");
            check(scheduler.wake(b_id).is_ok() && scheduler.state(b_id) == Some(TaskState::Ready), "B wake");
            write("SCHED b=woken event=synthetic\r\n");
        }
    }
    check(unsafe { read_volatile(addr_of!(STEP)) } == 5, "task order");
    check(scheduler.state(a_id) == Some(TaskState::Finished) && scheduler.state(b_id) == Some(TaskState::Finished), "finished states");
    check(scheduler.dispatch() == Ok(None) && scheduler.ready_count() == 0, "queue not empty");
    write("CHECK sequence=a1,b1,a2,a3,b2 registers_and_stacks=preserved queue=empty\r\n");
    write("PASS cooperative scheduler scenario\r\n"); halt();
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
