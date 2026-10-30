#![no_std]
#![no_main]

use core::{arch::{asm, global_asm}, panic::PanicInfo, ptr::{addr_of, read_volatile, write_volatile}};

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
    b unexpected_exception
    .balign 128
    // Vector slot 5, offset 0x280.
    b irq_el1_spx
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
irq_el1_spx:
    // All x0-x30 are saved before scratch use. SPSR_EL1 remains untouched.
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
    // QEMU virt GICv2 CPU interface: read interrupt acknowledge register.
    mov x1, #0x08010000
    ldr w0, [x1, #0x0c]
    and w2, w0, #0x3ff
    cmp w2, #27
    b.ne unexpected_exception
    adrp x2, IRQ_IAR
    add x2, x2, :lo12:IRQ_IAR
    str x0, [x2]
    // Deassert the virtual timer source before deactivating the GIC interrupt.
    msr CNTV_CTL_EL0, xzr
    isb
    dsb sy
    str w0, [x1, #0x10]
    dsb sy
    mrs x0, ELR_EL1
    adrp x1, probe_wait
    add x1, x1, :lo12:probe_wait
    cmp x0, x1
    b.ne unexpected_exception
    adrp x1, IRQ_PC
    add x1, x1, :lo12:IRQ_PC
    str x0, [x1]
    // In this tightly controlled test only, skip the deliberate wait loop.
    adrp x0, probe_resume
    add x0, x0, :lo12:probe_resume
    adrp x1, RESUME_PC
    add x1, x1, :lo12:RESUME_PC
    str x0, [x1]
    msr ELR_EL1, x0
    adrp x1, IRQ_COUNT
    add x1, x1, :lo12:IRQ_COUNT
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

    // A single controlled probe arms the timer with IRQ masked, restores x0,
    // unmasks IRQ and spins at one known instruction until the handler returns.
    .global irq_probe
irq_probe:
    sub sp, sp, #128
    stp x18, x19, [sp, #0]
    stp x20, x21, [sp, #16]
    stp x22, x23, [sp, #32]
    stp x24, x25, [sp, #48]
    stp x26, x27, [sp, #64]
    stp x28, x29, [sp, #80]
    str x30, [sp, #96]
    mov x1, sp
    str x1, [sp, #104]
    str x0, [sp, #112]
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
    ldr x0, [sp, #112]
    msr CNTV_TVAL_EL0, x0
    mov x0, #1
    msr CNTV_CTL_EL0, x0
    isb
    mov x0, #16
    msr daifclr, #2
    .global probe_wait
probe_wait:
    b probe_wait
    .global probe_resume
probe_resume:
    msr daifset, #2
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

#[no_mangle] static mut IRQ_IAR: u64 = 0;
#[no_mangle] static mut IRQ_PC: u64 = 0;
#[no_mangle] static mut RESUME_PC: u64 = 0;
#[no_mangle] static mut IRQ_COUNT: u64 = 0;
extern "C" { fn irq_probe(delta: u64) -> u64; static vector_table: u8; static probe_wait: u8; static probe_resume: u8; }

const GICD: usize = 0x0800_0000;
const GICC: usize = 0x0801_0000;
const TIMER_INTID: u32 = 27;
fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, message: &str) { if !ok { write("FAIL "); write(message); write("\r\n"); halt() } }
#[no_mangle]
pub extern "C" fn unexpected_exception() -> ! { write("FAIL unexpected exception or IRQ source\r\n"); halt() }

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut el: u64; let mut vbar: u64; let mut spsel: u64; let mut stack: u64; let mut freq: u64;
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) el);
        asm!("mrs {}, VBAR_EL1", out(reg) vbar);
        asm!("mrs {}, SPSel", out(reg) spsel);
        asm!("mov {}, sp", out(reg) stack);
        asm!("mrs {}, CNTFRQ_EL0", out(reg) freq);
    }
    check(el == 4 && spsel == 1 && stack & 15 == 0 && vbar == addr_of!(vector_table) as u64 && vbar & 0x7ff == 0, "EL1 VBAR or stack");
    let delta=freq/20;
    check(delta > 0 && delta <= i32::MAX as u64, "timer frequency");
    // These hard-coded MMIO addresses and INTID apply only to this exact QEMU
    // virt/gic-version=2 configuration, not a real Apple device.
    unsafe {
        write_volatile((GICD + 0x000) as *mut u32, 0); // distributor disabled
        write_volatile((GICC + 0x000) as *mut u32, 0); // CPU interface disabled
        write_volatile((GICD + 0x180) as *mut u32, 1 << TIMER_INTID); // disable PPI
        write_volatile((GICD + 0x280) as *mut u32, 1 << TIMER_INTID); // clear pending
        write_volatile((GICD + 0x400 + TIMER_INTID as usize) as *mut u8, 0x80); // priority
        write_volatile((GICC + 0x004) as *mut u32, 0xff); // priority mask
        write_volatile((GICD + 0x100) as *mut u32, 1 << TIMER_INTID); // enable PPI
        write_volatile((GICD + 0x000) as *mut u32, 1); // distributor
        write_volatile((GICC + 0x000) as *mut u32, 1); // CPU interface
        asm!("dsb sy", "isb");
    }
    write("BOOT el=1 gic=v2 virtual_timer_ppi=27 vector=el1-spx stack_alignment=16\r\n");
    // SAFETY: the assembly probe restores ABI-preserved registers; the handler
    // accepts only the expected GIC ID and a single known interrupted PC.
    let preserved=unsafe { irq_probe(delta) };
    let iar=unsafe { read_volatile(addr_of!(IRQ_IAR)) };
    let count=unsafe { read_volatile(addr_of!(IRQ_COUNT)) };
    let interrupted=unsafe { read_volatile(addr_of!(IRQ_PC)) };
    let resumed=unsafe { read_volatile(addr_of!(RESUME_PC)) };
    let mut timer_control:u64;
    unsafe { asm!("mrs {}, CNTV_CTL_EL0", out(reg) timer_control); }
    let active=unsafe { read_volatile((GICD+0x300) as *const u32) };
    check(count == 1 && iar & 0x3ff == u64::from(TIMER_INTID), "wrong timer interrupt");
    check(interrupted == addr_of!(probe_wait) as u64 && resumed == addr_of!(probe_resume) as u64, "IRQ resume PC");
    check(timer_control & 1 == 0 && active & (1 << TIMER_INTID) == 0, "timer disabled and GIC inactive");
    check(preserved == 1, "register or NZCV restoration");
    write("IRQ source=virtual-timer intid=27 count=1 ack=iar source=disabled eoi=written active=no\r\n");
    write("RETURN pc=controlled-probe-resume x0-x30=preserved sp=preserved nzcv=preserved\r\n");
    write("PASS controlled virtual timer IRQ\r\n");
    halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
