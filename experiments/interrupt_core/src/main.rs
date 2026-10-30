#![no_std]
#![no_main]

use core::{arch::{asm, global_asm}, panic::PanicInfo, ptr::{addr_of, read_volatile}};

global_asm!(r#"
    .section .text.boot, "ax"
    .global _start
_start:
    msr daifset, #0xf
    mrs x0, CurrentEL
    cmp x0, #4
    b.ne boot_halt
    mrs x0, SCTLR_EL1
    tbnz x0, #0, boot_halt
    tbnz x0, #2, boot_halt
    msr SPSel, #1
    ldr x0, =__bss_start
    ldr x1, =__bss_end
zero_bss:
    cmp x0, x1
    b.hs bss_done
    str xzr, [x0], #8
    b zero_bss
bss_done:
    ldr x0, =__stack_top
    mov sp, x0
    adrp x0, vector_table
    add x0, x0, :lo12:vector_table
    tst x0, #0x7ff
    b.ne boot_halt
    msr VBAR_EL1, x0
    isb
    mrs x1, VBAR_EL1
    cmp x0, x1
    b.ne boot_halt
    bl rust_main
boot_halt:
    wfe
    b boot_halt

    .section .text.vectors, "ax"
    .balign 2048
    .global vector_table
vector_table:
    // Vector slot 0, offset 0x000.
    b unexpected_exception
    .balign 128
    // Vector slot 1, offset 0x080.
    b unexpected_exception
    .balign 128
    // Vector slot 2, offset 0x100.
    b unexpected_exception
    .balign 128
    // Vector slot 3, offset 0x180.
    b unexpected_exception
    .balign 128
    // Vector slot 4, offset 0x200.
    b sync_el1_spx
    .balign 128
    // Vector slot 5, offset 0x280.
    b unexpected_exception
    .balign 128
    // Vector slot 6, offset 0x300.
    b unexpected_exception
    .balign 128
    // Vector slot 7, offset 0x380.
    b unexpected_exception
    .balign 128
    // Vector slot 8, offset 0x400.
    b unexpected_exception
    .balign 128
    // Vector slot 9, offset 0x480.
    b unexpected_exception
    .balign 128
    // Vector slot 10, offset 0x500.
    b unexpected_exception
    .balign 128
    // Vector slot 11, offset 0x580.
    b unexpected_exception
    .balign 128
    // Vector slot 12, offset 0x600.
    b unexpected_exception
    .balign 128
    // Vector slot 13, offset 0x680.
    b unexpected_exception
    .balign 128
    // Vector slot 14, offset 0x700.
    b unexpected_exception
    .balign 128
    // Vector slot 15, offset 0x780.
    b unexpected_exception
    .balign 128

    .section .text.handler, "ax"
sync_el1_spx:
    // Save all integer registers before any handler scratch use. 256 bytes
    // keeps SP 16-byte aligned. ELR and SPSR remain in system registers.
    sub sp, sp, #256
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, ESR_EL1
    lsr x1, x0, #26
    cmp x1, #0x3c
    b.ne unexpected_exception
    and x1, x0, #0xffff
    mov x2, #0x1234
    cmp x1, x2
    b.ne unexpected_exception
    adrp x1, TRAP_ESR
    add x1, x1, :lo12:TRAP_ESR
    str x0, [x1]
    mrs x0, ELR_EL1
    adrp x1, brk_site
    add x1, x1, :lo12:brk_site
    cmp x0, x1
    b.ne unexpected_exception
    adrp x1, TRAP_PC
    add x1, x1, :lo12:TRAP_PC
    str x0, [x1]
    add x0, x0, #4
    adrp x1, RESUME_PC
    add x1, x1, :lo12:RESUME_PC
    str x0, [x1]
    msr ELR_EL1, x0
    adrp x1, TRAP_COUNT
    add x1, x1, :lo12:TRAP_COUNT
    ldr x2, [x1]
    add x2, x2, #1
    str x2, [x1]
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    add sp, sp, #256
    eret

    // Preserve the ABI callee-saved registers, then seed all x0..x30 with
    // distinct sentinels around one controlled BRK and verify each on return.
    .global brk_probe
brk_probe:
    sub sp, sp, #128
    stp x18, x19, [sp, #0]
    stp x20, x21, [sp, #16]
    stp x22, x23, [sp, #32]
    stp x24, x25, [sp, #48]
    stp x26, x27, [sp, #64]
    stp x28, x29, [sp, #80]
    str x30, [sp, #96]
    mov x0, sp
    str x0, [sp, #104]
    mov x0, #0xa0000000
    msr NZCV, x0
    mov x0, #16
    mov x1, #17
    mov x2, #18
    mov x3, #19
    mov x4, #20
    mov x5, #21
    mov x6, #22
    mov x7, #23
    mov x8, #24
    mov x9, #25
    mov x10, #26
    mov x11, #27
    mov x12, #28
    mov x13, #29
    mov x14, #30
    mov x15, #31
    mov x16, #32
    mov x17, #33
    mov x18, #34
    mov x19, #35
    mov x20, #36
    mov x21, #37
    mov x22, #38
    mov x23, #39
    mov x24, #40
    mov x25, #41
    mov x26, #42
    mov x27, #43
    mov x28, #44
    mov x29, #45
    mov x30, #46
    .global brk_site
brk_site:
    brk #0x1234
    // Preserve x0 while capturing returned NZCV before comparisons alter it.
    str x0, [sp, #112]
    mrs x0, NZCV
    str x0, [sp, #120]
    ldr x0, [sp, #112]
    cmp x0, #16
    b.ne probe_failed
    cmp x1, #17
    b.ne probe_failed
    cmp x2, #18
    b.ne probe_failed
    cmp x3, #19
    b.ne probe_failed
    cmp x4, #20
    b.ne probe_failed
    cmp x5, #21
    b.ne probe_failed
    cmp x6, #22
    b.ne probe_failed
    cmp x7, #23
    b.ne probe_failed
    cmp x8, #24
    b.ne probe_failed
    cmp x9, #25
    b.ne probe_failed
    cmp x10, #26
    b.ne probe_failed
    cmp x11, #27
    b.ne probe_failed
    cmp x12, #28
    b.ne probe_failed
    cmp x13, #29
    b.ne probe_failed
    cmp x14, #30
    b.ne probe_failed
    cmp x15, #31
    b.ne probe_failed
    cmp x16, #32
    b.ne probe_failed
    cmp x17, #33
    b.ne probe_failed
    cmp x18, #34
    b.ne probe_failed
    cmp x19, #35
    b.ne probe_failed
    cmp x20, #36
    b.ne probe_failed
    cmp x21, #37
    b.ne probe_failed
    cmp x22, #38
    b.ne probe_failed
    cmp x23, #39
    b.ne probe_failed
    cmp x24, #40
    b.ne probe_failed
    cmp x25, #41
    b.ne probe_failed
    cmp x26, #42
    b.ne probe_failed
    cmp x27, #43
    b.ne probe_failed
    cmp x28, #44
    b.ne probe_failed
    cmp x29, #45
    b.ne probe_failed
    cmp x30, #46
    b.ne probe_failed
    mov x0, sp
    ldr x1, [sp, #104]
    cmp x0, x1
    b.ne probe_failed
    ldr x0, [sp, #120]
    mov x1, #0xa0000000
    cmp x0, x1
    b.ne probe_failed
    mov x0, #1
    b probe_restore
probe_failed:
    mov x0, #0
probe_restore:
    ldp x18, x19, [sp, #0]
    ldp x20, x21, [sp, #16]
    ldp x22, x23, [sp, #32]
    ldp x24, x25, [sp, #48]
    ldp x26, x27, [sp, #64]
    ldp x28, x29, [sp, #80]
    ldr x30, [sp, #96]
    add sp, sp, #128
    ret
"#);

#[no_mangle]
static mut TRAP_ESR: u64 = 0;
#[no_mangle]
static mut TRAP_PC: u64 = 0;
#[no_mangle]
static mut RESUME_PC: u64 = 0;
#[no_mangle]
static mut TRAP_COUNT: u64 = 0;
extern "C" { fn brk_probe() -> u64; static vector_table: u8; static brk_site: u8; }

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, message: &str) { if !ok { write("FAIL "); write(message); write("\r\n"); halt() } }

// The controlled demo never returns from an unexpected vector or syndrome.
#[no_mangle]
pub extern "C" fn unexpected_exception() -> ! { write("FAIL unexpected exception\r\n"); halt() }

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut el: u64;
    let mut vbar: u64;
    let mut spsel: u64;
    let mut stack: u64;
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) el);
        asm!("mrs {}, VBAR_EL1", out(reg) vbar);
        asm!("mrs {}, SPSel", out(reg) spsel);
        asm!("mov {}, sp", out(reg) stack);
    }
    let expected_vbar = addr_of!(vector_table) as u64;
    check(el == 4 && spsel == 1 && stack & 15 == 0 && vbar == expected_vbar && vbar & 0x7ff == 0, "EL1 VBAR or stack");
    write("BOOT el=1 stack=sp_el1 stack_alignment=16 vector_alignment=2048 vbar_readback=ok\r\n");
    // SAFETY: the assembly probe preserves the AArch64 callee-saved registers;
    // the controlled vector validates the exact BRK site before resuming.
    let preserved = unsafe { brk_probe() };
    let esr = unsafe { read_volatile(addr_of!(TRAP_ESR)) };
    let pc = unsafe { read_volatile(addr_of!(TRAP_PC)) };
    let resume = unsafe { read_volatile(addr_of!(RESUME_PC)) };
    let count = unsafe { read_volatile(addr_of!(TRAP_COUNT)) };
    check(count == 1 && esr >> 26 == 0x3c && esr & (1 << 25) != 0 && esr & 0xffff == 0x1234, "BRK syndrome/count");
    check(pc == addr_of!(brk_site) as u64 && pc.checked_add(4) == Some(resume), "exception return PC");
    check(preserved == 1, "register or NZCV restoration");
    write("TRAP kind=brk64 immediate=0x1234 count=1 vector=current-el-spx\r\n");
    write("RETURN pc=next-instruction x0-x30=preserved sp=preserved nzcv=preserved\r\n");
    write("PASS controlled synchronous exception\r\n");
    halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
