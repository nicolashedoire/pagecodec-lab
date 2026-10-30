#![no_std]
#![no_main]

use core::{arch::{asm, global_asm}, panic::PanicInfo, ptr::{addr_of, addr_of_mut, read_volatile}};

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
// Linker-provided symbols can coincide with the end of a Rust static. Do not
// let compiler provenance assumptions turn physical boundary comparisons into
// compile-time assumptions about distinct Rust objects.
fn physical_address(value: usize) -> usize {
    let observed: usize;
    unsafe { asm!("mov {observed}, {value}", observed = out(reg) observed,
                  value = in(reg) value, options(nomem, nostack, preserves_flags)); }
    observed
}
fn write_hex(value: usize) {
    const HEX: &[u8;16]=b"0123456789abcdef";
    write("0x");
    for shift in (0..16).rev() {
        let b=HEX[(value >> (shift*4)) & 15];
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

#[no_mangle]
extern "C" fn task_a() -> ! {
    let mut sentinel = [0u64; 2];
    let p = sentinel.as_mut_ptr();
    check(object_on_stack(addr_of!(A_STACK), p as usize, 16), "A sentinel not on stack");
    unsafe { p.write_volatile(0x1122_3344_5566_7788); p.add(1).write_volatile(0x0102_0304_0506_0708); }
    check(stack_contains(addr_of!(A_STACK), current_sp()), "A stack bounds");
    tick(0); write("TASK a stage=1 stack=a\r\n");
    let result = unsafe { yield_probe(addr_of_mut!(A_CONTEXT), addr_of!(B_CONTEXT), 0x100) };
    check(result == 1, "A nonvolatile registers or SP");
    check(unsafe { read_volatile(sentinel.as_ptr()) } == 0x1122_3344_5566_7788 && unsafe { read_volatile(sentinel.as_ptr().add(1)) } == 0x0102_0304_0506_0708, "A stack contents");
    check(stack_contains(addr_of!(A_STACK), current_sp()), "A resumed stack bounds");
    tick(2); write("TASK a stage=2 registers=preserved stack_contents=preserved\r\n");
    unsafe { yield_probe(addr_of_mut!(A_CONTEXT), addr_of!(B_CONTEXT), 0x200); }
    write("FAIL unexpected A resume\r\n"); halt()
}

#[no_mangle]
extern "C" fn task_b() -> ! {
    let mut sentinel = [0u64; 2];
    let p = sentinel.as_mut_ptr();
    check(object_on_stack(addr_of!(B_STACK), p as usize, 16), "B sentinel not on stack");
    unsafe { p.write_volatile(0x8877_6655_4433_2211); p.add(1).write_volatile(0x0807_0605_0403_0201); }
    check(stack_contains(addr_of!(B_STACK), current_sp()), "B stack bounds");
    tick(1); write("TASK b stage=1 stack=b\r\n");
    let result = unsafe { yield_probe(addr_of_mut!(B_CONTEXT), addr_of!(A_CONTEXT), 0x300) };
    check(result == 1, "B nonvolatile registers or SP");
    check(unsafe { read_volatile(sentinel.as_ptr()) } == 0x8877_6655_4433_2211 && unsafe { read_volatile(sentinel.as_ptr().add(1)) } == 0x0807_0605_0403_0201, "B stack contents");
    check(stack_contains(addr_of!(B_STACK), current_sp()), "B resumed stack bounds");
    tick(3); write("TASK b stage=2 registers=preserved stack_contents=preserved\r\n");
    unsafe { yield_probe(addr_of_mut!(B_CONTEXT), addr_of!(MAIN_CONTEXT), 0x400); }
    write("FAIL unexpected B resume\r\n"); halt()
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let a = physical_address(addr_of!(A_STACK) as usize);
    let b = physical_address(addr_of!(B_STACK) as usize);
    let main_start = physical_address(addr_of!(__bss_end) as usize);
    let main_top = physical_address(addr_of!(__stack_top) as usize);
    let main_sp = current_sp();
    let a_end = a.checked_add(STACK_BYTES);
    let b_end = b.checked_add(STACK_BYTES);
    let valid = match (a_end,b_end) {
        (Some(a_end),Some(b_end)) => a % 16 == 0 && b % 16 == 0 && main_start % 16 == 0 && main_top % 16 == 0
            && (a_end <= b || b_end <= a)
            && (a_end <= main_start || main_top <= a) && (b_end <= main_start || main_top <= b)
            // AArch64 permits SP at the top of an empty descending stack.
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
    check(!stack_position(main_start, main_top, main_start), "empty stack bottom boundary");
    check(main_top.checked_add(16).is_some_and(|outside| !stack_position(main_start, main_top, outside)), "outside main stack boundary");
    write("CHECK stack_boundaries=top_allowed_bottom_outside_rejected opaque_linker_addresses=yes\r\n");
    unsafe {
        (*addr_of_mut!(A_CONTEXT)).words[0] = (a + STACK_BYTES) as u64;
        (*addr_of_mut!(A_CONTEXT)).words[12] = task_a as *const () as usize as u64;
        (*addr_of_mut!(B_CONTEXT)).words[0] = (b + STACK_BYTES) as u64;
        (*addr_of_mut!(B_CONTEXT)).words[12] = task_b as *const () as usize as u64;
    }
    let result = unsafe { yield_probe(addr_of_mut!(MAIN_CONTEXT), addr_of!(A_CONTEXT), 0x500) };
    check(result == 1, "main nonvolatile registers or SP");
    check(unsafe { read_volatile(addr_of!(STEP)) } == 4, "completion order");
    write("CHECK main_registers=preserved stacks=disjoint_aligned sequence=a1,b1,a2,b2\r\n");
    write("PASS cooperative stack switching\r\n"); halt()
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
