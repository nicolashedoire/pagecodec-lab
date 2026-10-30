#![no_std]
#![no_main]

use core::{arch::{asm, global_asm}, panic::PanicInfo, ptr::{addr_of, addr_of_mut, read_volatile, write_volatile}};

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
    tbnz x0, #12, boot_halt
    tbnz x0, #25, boot_halt
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
    bl rust_main
boot_halt:
    wfe
    b boot_halt
"#);

global_asm!(r#"
    .section .text.vectors, "ax"
    .balign 2048
    .global user_vector_table
user_vector_table:
    b unexpected_exception // current EL SP0 sync
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception // current EL SPx sync
    .balign 128
    b timer_irq            // current EL SPx IRQ
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b lower_el_sync        // lower EL AArch64 sync
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128

    .section .text.user_handler, "ax"
timer_irq:
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
    mov x1, #0x08010000
    ldr w0, [x1, #0x0c]
    and w2, w0, #0x3ff
    cmp w2, #27
    b.ne irq_fatal
    msr CNTV_CTL_EL0, xzr
    isb
    dsb sy
    str w0, [x1, #0x10]
    dsb sy
    adrp x1, IRQ_COUNT
    add x1, x1, :lo12:IRQ_COUNT
    ldr x0, [x1]
    cbnz x0, irq_fatal
    mov x0, #1
    str x0, [x1]
    mrs x0, ELR_EL1
    adrp x1, IRQ_PC
    add x1, x1, :lo12:IRQ_PC
    str x0, [x1]
    mrs x0, SPSR_EL1
    adrp x1, IRQ_SPSR
    add x1, x1, :lo12:IRQ_SPSR
    str x0, [x1]
    add x0, sp, #256
    adrp x1, IRQ_SP
    add x1, x1, :lo12:IRQ_SP
    str x0, [x1]
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
irq_fatal:
    msr daifset, #0xf
1:  wfe
    b 1b

    .global timer_wait
timer_wait:
    msr daifclr, #2
    .global timer_wait_begin
timer_wait_begin:
    adrp x0, IRQ_COUNT
    add x0, x0, :lo12:IRQ_COUNT
    ldr x0, [x0]
    cbz x0, timer_wait_begin
    msr daifset, #2
    .global timer_wait_end
timer_wait_end:
    ret

lower_el_sync:
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
    mrs x0, SPSR_EL1
    and x1, x0, #0xf
    cbnz x1, unexpected_exception
    mrs x0, SP_EL0
    adrp x1, USER_STACK
    add x1, x1, :lo12:USER_STACK
    add x1, x1, #1, lsl #12
    sub x1, x1, #16
    cmp x0, x1
    b.ne unexpected_exception
    mrs x0, ESR_EL1
    tbz x0, #25, unexpected_exception
    lsr x1, x0, #26
    cmp x1, #0x24
    b.eq user_data_abort
    cmp x1, #0x15
    b.eq user_svc
    b unexpected_exception

user_data_abort:
    tbnz x0, #6, unexpected_exception
    tbnz x0, #7, unexpected_exception
    tbnz x0, #10, unexpected_exception
    and x1, x0, #0x3f
    cmp x1, #0x0f
    b.ne unexpected_exception
    mrs x1, FAR_EL1
    adrp x2, KERNEL_DATA
    add x2, x2, :lo12:KERNEL_DATA
    cmp x1, x2
    b.ne unexpected_exception
    ldr x3, [sp, #16]
    cmp x3, x2
    b.ne unexpected_exception
    mrs x3, ELR_EL1
    adrp x4, user_denied_load
    add x4, x4, :lo12:user_denied_load
    cmp x3, x4
    b.ne unexpected_exception
    adrp x4, ABORT_ESR
    add x4, x4, :lo12:ABORT_ESR
    str x0, [x4]
    adrp x4, ABORT_FAR
    add x4, x4, :lo12:ABORT_FAR
    str x1, [x4]
    adrp x4, ABORT_PC
    add x4, x4, :lo12:ABORT_PC
    str x3, [x4]
    add x3, x3, #4
    msr ELR_EL1, x3
    adrp x4, ABORT_COUNT
    add x4, x4, :lo12:ABORT_COUNT
    ldr x5, [x4]
    add x5, x5, #1
    str x5, [x4]
    b restore_user

user_svc:
    and x1, x0, #0xffff
    mov x2, #0x1234
    cmp x1, x2
    b.eq first_svc
    mov x2, #0x5678
    cmp x1, x2
    b.eq final_svc
    b unexpected_exception
first_svc:
    adrp x1, ABORT_COUNT
    add x1, x1, :lo12:ABORT_COUNT
    ldr x2, [x1]
    cmp x2, #1
    b.ne unexpected_exception
    ldr x2, [sp, #0]
    cmp x2, #0x42
    b.ne unexpected_exception
    ldr x2, [sp, #8]
    cmp x2, #0x123
    b.ne unexpected_exception
    mrs x2, ELR_EL1
    adrp x3, user_after_first_svc
    add x3, x3, :lo12:user_after_first_svc
    cmp x2, x3
    b.ne unexpected_exception
    adrp x3, SVC_ESR
    add x3, x3, :lo12:SVC_ESR
    str x0, [x3]
    adrp x3, SVC_PC
    add x3, x3, :lo12:SVC_PC
    str x2, [x3]
    mrs x2, SP_EL0
    adrp x3, SVC_SP
    add x3, x3, :lo12:SVC_SP
    str x2, [x3]
    mrs x2, SPSR_EL1
    adrp x3, SVC_SPSR
    add x3, x3, :lo12:SVC_SPSR
    str x2, [x3]
    adrp x3, SVC_COUNT
    add x3, x3, :lo12:SVC_COUNT
    ldr x2, [x3]
    add x2, x2, #1
    str x2, [x3]
    mov x2, #0x99
    str x2, [sp, #0]
    b restore_user
final_svc:
    adrp x1, SVC_COUNT
    add x1, x1, :lo12:SVC_COUNT
    ldr x2, [x1]
    cmp x2, #1
    b.ne unexpected_exception
    ldr x2, [sp, #0]
    mov x3, #0xbeef
    cmp x2, x3
    b.ne unexpected_exception
    mrs x2, ELR_EL1
    adrp x3, user_after_final_svc
    add x3, x3, :lo12:user_after_final_svc
    cmp x2, x3
    b.ne unexpected_exception
    adrp x3, FINAL_SVC_ESR
    add x3, x3, :lo12:FINAL_SVC_ESR
    str x0, [x3]
    b finish

restore_user:
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

    .global enter_el0
enter_el0:
    // x0 = user stack top, x1 = user entry, x2 = protected kernel data VA.
    msr SP_EL0, x0
    msr ELR_EL1, x1
    mov x3, #0x3c0
    msr SPSR_EL1, x3
    eret

    .section .usertext, "ax"
    .global user_entry
user_entry:
    mov x1, #0x123
    mov x3, #0x77
    sub sp, sp, #16
    mov x4, #0xabc
    str x4, [sp]
    .global user_denied_load
user_denied_load:
    ldr x3, [x2]
    cmp x3, #0x77
    b.ne user_failed
    mov x0, #0x42
    svc #0x1234
    .global user_after_first_svc
user_after_first_svc:
    cmp x0, #0x99
    b.ne user_failed
    cmp x1, #0x123
    b.ne user_failed
    ldr x4, [sp]
    cmp x4, #0xabc
    b.ne user_failed
    mov x0, #0xbeef
    svc #0x5678
    .global user_after_final_svc
user_after_final_svc:
user_failed:
    svc #0x7bad
    b user_failed
"#);

const PAGE: usize = 4096;
const ENTRIES: usize = 512;
const RAM_REGION: usize = 0x4000_0000;
const REGION_SIZE: usize = 2 * 1024 * 1024;
const UART: usize = 0x0900_0000;
const GICD: usize = 0x0800_0000;
const GICC: usize = 0x0801_0000;
const UXN: u64 = 1 << 54;
const PXN: u64 = 1 << 53;
const AF: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 3 << 8;
const EL1_READONLY: u64 = 1 << 7;
const EL0_ACCESS: u64 = 1 << 6;
const NORMAL_INDEX: u64 = 1 << 2;
const TABLE_OR_PAGE: u64 = 3;
const PA_MASK: u64 = 0x0000_ffff_ffff_f000;
const EXPECTED: u64 = 0x0123_4567_89ab_cdef;

#[repr(C, align(4096))]
struct Table([u64; ENTRIES]);
static mut L0: Table = Table([0; ENTRIES]);
static mut L1: Table = Table([0; ENTRIES]);
static mut L2_LOW: Table = Table([0; ENTRIES]);
static mut L2_RAM: Table = Table([0; ENTRIES]);
static mut L3_UART: Table = Table([0; ENTRIES]);
static mut L3_GIC: Table = Table([0; ENTRIES]);
static mut L3_RAM: Table = Table([0; ENTRIES]);
static mut PROBE: u64 = 0;
#[repr(C, align(4096))]
struct UserStack([u8; PAGE]);
#[no_mangle]
#[link_section = ".userstack"]
static mut USER_STACK: UserStack = UserStack([0; PAGE]);
#[no_mangle] static mut KERNEL_DATA:u64 = EXPECTED;
#[no_mangle] static mut ABORT_ESR:u64 = 0;
#[no_mangle] static mut ABORT_FAR:u64 = 0;
#[no_mangle] static mut ABORT_PC:u64 = 0;
#[no_mangle] static mut ABORT_COUNT:u64 = 0;
#[no_mangle] static mut SVC_ESR:u64 = 0;
#[no_mangle] static mut SVC_PC:u64 = 0;
#[no_mangle] static mut SVC_SP:u64 = 0;
#[no_mangle] static mut SVC_SPSR:u64 = 0;
#[no_mangle] static mut SVC_COUNT:u64 = 0;
#[no_mangle] static mut FINAL_SVC_ESR:u64 = 0;
#[no_mangle] static mut IRQ_COUNT:u64 = 0;
#[no_mangle] static mut IRQ_PC:u64 = 0;
#[no_mangle] static mut IRQ_SPSR:u64 = 0;
#[no_mangle] static mut IRQ_SP:u64 = 0;
extern "C" {
    static __image_start: u8;
    static __image_end: u8;
    static __text_end: u8;
    static __rodata_end: u8;
    static __usertext_start: u8;
    static __usertext_end: u8;
    static __userstack_start: u8;
    static __userstack_end: u8;
    static __stack_top: u8;
    static __bss_end: u8;
    static user_vector_table: u8;
    static user_entry: u8;
    static user_denied_load: u8;
    static user_after_first_svc: u8;
    static timer_wait_begin: u8;
    static timer_wait_end: u8;
    fn enter_el0(stack_top:usize, entry:usize, protected:usize) -> !;
    fn timer_wait();
}

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (UART as *const u32).add(6).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (UART as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, message: &str) { if !ok { write("FAIL "); write(message); write("\r\n"); halt() } }
#[no_mangle]
pub extern "C" fn unexpected_exception() -> ! { write("FAIL unexpected exception\r\n"); halt() }
fn physical_address(value:usize) -> usize {
    let observed:usize;
    unsafe {asm!("mov {out}, {input}",out=out(reg) observed,input=in(reg) value,options(nomem,nostack,preserves_flags));}
    observed
}
fn table_addr(p: *mut Table, start: usize, end: usize) -> u64 {
    let a = p as usize;
    check(a % PAGE == 0 && a >= start && a.checked_add(PAGE).is_some_and(|n| n <= end), "table alignment or location");
    a as u64
}
// SAFETY: caller supplies a distinct statically allocated table and a checked index.
unsafe fn set(table: *mut Table, index: usize, descriptor: u64) {
    assert!(index < ENTRIES);
    let entries = table.cast::<u64>();
    entries.add(index).write_volatile(descriptor);
}
fn query(address: usize) -> u64 {
    let par: u64;
    // SAFETY: AT performs translation only; an unmapped address returns an error in PAR_EL1.
    unsafe { asm!("at s1e1r, {address}", "isb", "mrs {par}, PAR_EL1", address=in(reg) address, par=out(reg) par); }
    par
}
fn query_write(address: usize) -> u64 {
    let par: u64;
    unsafe { asm!("at s1e1w, {address}", "isb", "mrs {par}, PAR_EL1", address=in(reg) address, par=out(reg) par); }
    par
}
fn query_user_read(address: usize) -> u64 {
    let par: u64;
    unsafe { asm!("at s1e0r, {address}", "isb", "mrs {par}, PAR_EL1", address=in(reg) address, par=out(reg) par); }
    par
}
fn query_user_write(address: usize) -> u64 {
    let par: u64;
    unsafe { asm!("at s1e0w, {address}", "isb", "mrs {par}, PAR_EL1", address=in(reg) address, par=out(reg) par); }
    par
}
fn mapped_identity(address: usize) -> bool {
    let par=query(address);
    par & 1 == 0 && (par & PA_MASK) == ((address as u64) & PA_MASK)
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut el:u64; let mut sctlr:u64; let mut sp:u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el); asm!("mrs {}, SCTLR_EL1", out(reg) sctlr); asm!("mov {}, sp", out(reg) sp); }
    let start=physical_address(addr_of!(__image_start) as usize);
    let end=physical_address(addr_of!(__image_end) as usize);
    let text_end=physical_address(addr_of!(__text_end) as usize);
    let user_start=physical_address(addr_of!(__usertext_start) as usize);
    let user_end=physical_address(addr_of!(__usertext_end) as usize);
    let ro_end=physical_address(addr_of!(__rodata_end) as usize);
    let user_stack_start=physical_address(addr_of!(__userstack_start) as usize);
    let user_stack_end=physical_address(addr_of!(__userstack_end) as usize);
    let user_stack=physical_address(addr_of_mut!(USER_STACK) as usize);
    let kernel_top=physical_address(addr_of!(__stack_top) as usize);
    let vector=physical_address(addr_of!(user_vector_table) as usize);
    let entry=physical_address(addr_of!(user_entry) as usize);
    let protected=physical_address(addr_of_mut!(KERNEL_DATA) as usize);
    let data_start=ro_end;
    check(el == 4 && sctlr & ((1<<0)|(1<<2)|(1<<12)|(1<<25)) == 0 && sp & 15 == 0, "unexpected boot state");
    check(start % PAGE == 0 && end % PAGE == 0 && start >= RAM_REGION && end <= RAM_REGION + REGION_SIZE && start < end, "image range");
    check(start < text_end && text_end <= user_start && user_start < user_end && user_end <= ro_end && ro_end < user_stack_start
          && user_stack_start < user_stack_end && user_stack_end <= end
          && [text_end,user_start,user_end,ro_end,user_stack_start,user_stack_end].iter().all(|v| v % PAGE == 0), "section boundaries");
    check(user_stack_start.checked_add(PAGE) == Some(user_stack_end) && user_stack == user_stack_start, "user stack page");
    check(data_start < kernel_top && sp as usize > data_start && (sp as usize) <= kernel_top && kernel_top <= user_stack_start, "kernel stack bounds");
    check(protected >= data_start && protected.checked_add(8).is_some_and(|n| n <= kernel_top), "kernel data range");
    check(entry >= user_start && entry < user_end && vector % 2048 == 0 && vector >= start && vector.checked_add(2048).is_some_and(|n| n <= text_end), "entry or vector bounds");
    // The dedicated NOLOAD page is outside the startup BSS clearing range.
    // Explicitly clear and verify it before exposing it to EL0.
    let user_stack_bytes=addr_of_mut!(USER_STACK).cast::<u8>();
    for i in 0..PAGE { unsafe {write_volatile(user_stack_bytes.add(i), 0)}; }
    for i in 0..PAGE { check(unsafe {read_volatile(user_stack_bytes.add(i))} == 0, "user stack not zeroed"); }
    unsafe { asm!("msr VBAR_EL1, {}", "isb", in(reg) vector); }
    let mut vbar:u64;
    unsafe { asm!("mrs {}, VBAR_EL1", out(reg) vbar); }
    check(vbar == vector as u64, "VBAR readback");
    let l0=addr_of_mut!(L0); let l1=addr_of_mut!(L1);
    let l2low=addr_of_mut!(L2_LOW); let l2ram=addr_of_mut!(L2_RAM);
    let l3uart=addr_of_mut!(L3_UART); let l3gic=addr_of_mut!(L3_GIC); let l3ram=addr_of_mut!(L3_RAM);
    let a0=table_addr(l0,start,end); let a1=table_addr(l1,start,end);
    let a2low=table_addr(l2low,start,end); let a2ram=table_addr(l2ram,start,end);
    let a3uart=table_addr(l3uart,start,end); let a3gic=table_addr(l3gic,start,end); let a3ram=table_addr(l3ram,start,end);
    // All tables start zeroed in BSS. Entries outside these two L3 regions remain invalid.
    unsafe {
        set(l0,0,a1|TABLE_OR_PAGE);
        set(l1,0,a2low|TABLE_OR_PAGE);
        set(l1,1,a2ram|TABLE_OR_PAGE);
        set(l2low,(UART >> 21)&511,a3uart|TABLE_OR_PAGE);
        set(l2low,(GICD >> 21)&511,a3gic|TABLE_OR_PAGE);
        set(l2ram,(RAM_REGION >> 21)&511,a3ram|TABLE_OR_PAGE);
        set(l3uart,(UART >> 12)&511,(UART as u64)|AF|PXN|UXN|TABLE_OR_PAGE);
        set(l3gic,(GICD >> 12)&511,(GICD as u64)|AF|PXN|UXN|TABLE_OR_PAGE);
        set(l3gic,(GICC >> 12)&511,(GICC as u64)|AF|PXN|UXN|TABLE_OR_PAGE);
        for address in (start..end).step_by(PAGE) {
            let flags=if address < text_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|UXN }
                else if address >= user_start && address < user_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|EL0_ACCESS|PXN }
                else if address < ro_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|PXN|UXN }
                else if address >= user_stack_start && address < user_stack_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL0_ACCESS|PXN|UXN }
                else { AF|INNER_SHAREABLE|NORMAL_INDEX|PXN|UXN };
            set(l3ram,(address >> 12)&511,(address as u64)|flags|TABLE_OR_PAGE);
        }
    }
    let addresses=[a0,a1,a2low,a2ram,a3uart,a3gic,a3ram];
    for i in 0..addresses.len() {
        for j in 0..i { check(addresses[i] != addresses[j], "overlapping tables"); }
    }
    // The configured data and device descriptors are marked execute-never;
    // this reads the entries, not a deliberate attempt to execute those pages.
    let data_entry=unsafe { l3ram.cast::<u64>().add((data_start >> 12)&511).read_volatile() };
    let uart_entry=unsafe { l3uart.cast::<u64>().add((UART >> 12)&511).read_volatile() };
    let gicd_entry=unsafe { l3gic.cast::<u64>().add((GICD >> 12)&511).read_volatile() };
    let gicc_entry=unsafe { l3gic.cast::<u64>().add((GICC >> 12)&511).read_volatile() };
    check(data_entry & (PXN|UXN) == (PXN|UXN) && uart_entry & (PXN|UXN) == (PXN|UXN)
          && gicd_entry & (PXN|UXN|(7<<2)) == (PXN|UXN) && gicc_entry & (PXN|UXN|(7<<2)) == (PXN|UXN), "execute-never descriptors");
    write("BOOT el=1 pages=4k image=bounded uart=one-page tables=aligned\r\n");
    // MAIR Attr0: Device-nGnRnE; Attr1: Normal non-cacheable (0x44).
    // TCR: 48-bit TTBR0 VA, 4K granule, inner-shareable NC walks, 32-bit PA,
    // TTBR1 walks disabled. Interrupts remain masked; only a single CPU runs.
    let mair=0x4400u64;
    let tcr=16u64|(3<<12)|(1<<23);
    unsafe {
        asm!("dsb sy", "msr MAIR_EL1, {mair}", "msr TCR_EL1, {tcr}", "msr TTBR0_EL1, {root}",
             "isb", "tlbi vmalle1", "dsb sy", "isb", "mrs x9, SCTLR_EL1", "orr x9, x9, #1",
             "msr SCTLR_EL1, x9", "isb", mair=in(reg) mair, tcr=in(reg) tcr, root=in(reg) a0, out("x9") _);
    }
    unsafe { asm!("mrs {}, SCTLR_EL1", out(reg) sctlr); }
    check(sctlr & 1 != 0, "MMU not enabled");
    // The identity mapping retains the original static pointer provenance.
    let probe=addr_of_mut!(PROBE);
    unsafe { write_volatile(probe, EXPECTED); }
    check(unsafe { read_volatile(probe) } == EXPECTED, "probe readback");
    check(mapped_identity(start) && mapped_identity(probe as usize) && mapped_identity((sp as usize)-8) && mapped_identity(UART), "identity translation");
    check(query(start-PAGE)&1 != 0 && query(end)&1 != 0 && query(UART+PAGE)&1 != 0
          && query(GICD+PAGE)&1 != 0 && query(GICC+PAGE)&1 != 0, "neighbor unexpectedly mapped");
    check(query_write(start)&1 != 0 && query_write(probe as usize)&1 == 0
        && query_user_read(probe as usize)&1 != 0 && query_user_read(UART)&1 != 0, "translation access permissions");
    check(mapped_identity(entry) && mapped_identity(user_stack_start) && mapped_identity(protected)
          && mapped_identity(GICD) && mapped_identity(GICC), "user or kernel identity");
    check(query_user_read(entry)&1 == 0 && query_user_write(entry)&1 != 0
          && query_user_read(user_stack_start)&1 == 0 && query_user_write(user_stack_start)&1 == 0
          && query_user_read(protected)&1 != 0 && query_user_write(protected)&1 != 0
          && query_user_read(GICD)&1 != 0 && query_user_write(GICD)&1 != 0
          && query_user_read(GICC)&1 != 0 && query_user_write(GICC)&1 != 0
          && query_user_read(UART)&1 != 0 && query_user_write(UART)&1 != 0, "EL0 permissions");
    let user_code_entry=unsafe {l3ram.cast::<u64>().add((entry >> 12)&511).read_volatile()};
    let user_stack_entry=unsafe {l3ram.cast::<u64>().add((user_stack_start >> 12)&511).read_volatile()};
    check(user_code_entry & (EL0_ACCESS|EL1_READONLY|PXN|UXN) == (EL0_ACCESS|EL1_READONLY|PXN)
          && user_stack_entry & (EL0_ACCESS|EL1_READONLY|PXN|UXN) == (EL0_ACCESS|PXN|UXN), "user descriptors");
    write("MMU enabled=yes kernel=privileged user_code=ro user_stack=rw uart_gic=privileged\r\n");
    write("CHECK at_el0 kernel_read=denied kernel_write=denied user_code=read_only user_stack=rw zeroed=yes\r\n");
    unsafe { enter_el0(user_stack_end,entry,protected); }
}

#[no_mangle]
pub extern "C" fn finish() -> ! {
    let mut el:u64; unsafe { asm!("mrs {}, CurrentEL",out(reg) el); }
    let abort_esr=unsafe {read_volatile(addr_of!(ABORT_ESR))};
    let abort_far=unsafe {read_volatile(addr_of!(ABORT_FAR))};
    let abort_pc=unsafe {read_volatile(addr_of!(ABORT_PC))};
    let abort_count=unsafe {read_volatile(addr_of!(ABORT_COUNT))};
    let svc_esr=unsafe {read_volatile(addr_of!(SVC_ESR))};
    let svc_pc=unsafe {read_volatile(addr_of!(SVC_PC))};
    let svc_sp=unsafe {read_volatile(addr_of!(SVC_SP))};
    let svc_spsr=unsafe {read_volatile(addr_of!(SVC_SPSR))};
    let svc_count=unsafe {read_volatile(addr_of!(SVC_COUNT))};
    let final_esr=unsafe {read_volatile(addr_of!(FINAL_SVC_ESR))};
    let stack=physical_address(addr_of_mut!(USER_STACK) as usize);
    let protected=physical_address(addr_of_mut!(KERNEL_DATA) as usize);
    check(el==4 && abort_count==1 && svc_count==1, "exception counts or EL1");
    check(abort_esr>>26==0x24 && abort_esr&(1<<25)!=0 && abort_esr&(1<<6)==0 && abort_esr&(1<<7)==0 && abort_esr&(1<<10)==0 && abort_esr&0x3f==0x0f, "lower EL abort syndrome");
    check(abort_far==protected as u64 && abort_pc==addr_of!(user_denied_load) as u64, "lower EL fault address");
    check(svc_esr>>26==0x15 && svc_esr&(1<<25)!=0 && svc_esr&0xffff==0x1234 && svc_pc==addr_of!(user_after_first_svc) as u64, "first SVC syndrome or PC");
    check(svc_sp==(stack+PAGE-16) as u64 && svc_spsr&0xf==0 && svc_spsr&0x3c0==0x3c0, "EL0 SP or SPSR");
    check(final_esr>>26==0x15 && final_esr&0xffff==0x5678, "final SVC syndrome");
    check(unsafe {read_volatile(addr_of!(KERNEL_DATA))}==EXPECTED, "kernel data changed");
    let user_slot=unsafe {addr_of!(USER_STACK).cast::<u8>().add(PAGE-16).cast::<u64>()};
    check(unsafe {read_volatile(user_slot)}==0xabc, "user stack value");
    write("ABORT el0_read_kernel=denied ec=0x24 dfsc=0x0f far=kernel elr=load count=1\r\n");
    write("SVC first=0x1234 response=0x99 returned_to_el0=yes final=0x5678\r\n");
    write("CHECK el0_sp=dedicated spsr=el0t kernel_data=unchanged user_stack=preserved\r\n");
    let mut freq:u64; unsafe {asm!("mrs {}, CNTFRQ_EL0",out(reg) freq);}
    let delta=freq/100;
    check(delta>0 && delta<=i32::MAX as u64 && unsafe {read_volatile(addr_of!(IRQ_COUNT))}==0, "timer setup");
    unsafe {
        write_volatile((GICD+0x000) as *mut u32,0); write_volatile((GICC+0x000) as *mut u32,0);
        write_volatile((GICD+0x180) as *mut u32,1<<27); write_volatile((GICD+0x280) as *mut u32,1<<27);
        write_volatile((GICD+0x400+27) as *mut u8,0x80); write_volatile((GICC+0x004) as *mut u32,0xff);
        write_volatile((GICD+0x100) as *mut u32,1<<27); write_volatile((GICD+0x000) as *mut u32,1); write_volatile((GICC+0x000) as *mut u32,1);
        asm!("dsb sy", "isb");
        asm!("msr CNTV_TVAL_EL0, {}",in(reg) delta);
        asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);
        asm!("isb");
        timer_wait();
    }
    let irq_count=unsafe {read_volatile(addr_of!(IRQ_COUNT))};
    let irq_pc=unsafe {read_volatile(addr_of!(IRQ_PC))};
    let irq_spsr=unsafe {read_volatile(addr_of!(IRQ_SPSR))};
    let irq_sp=unsafe {read_volatile(addr_of!(IRQ_SP))};
    let wait_begin=physical_address(addr_of!(timer_wait_begin) as usize);
    let wait_end=physical_address(addr_of!(timer_wait_end) as usize);
    let kernel_stack_start=physical_address(addr_of!(__bss_end) as usize);
    let kernel_stack_top=physical_address(addr_of!(__stack_top) as usize);
    let mut ctl:u64; unsafe {asm!("mrs {}, CNTV_CTL_EL0",out(reg) ctl);}
    let active=unsafe {read_volatile((GICD+0x300) as *const u32)};
    check(irq_count==1 && irq_pc as usize >= wait_begin && (irq_pc as usize) < wait_end && irq_pc%4==0, "timer count or interrupted PC");
    check(irq_spsr&0xf==5 && irq_spsr&(1<<7)==0 && irq_sp%16==0 && irq_sp as usize>kernel_stack_start && (irq_sp as usize)<=kernel_stack_top, "timer interrupted state");
    check(ctl&1==0 && active&(1<<27)==0, "timer or GIC active");
    write("TIMER irq=ppi27 count=1 interrupted=el1_wait resumed=yes disabled=yes gic=inactive\r\n");
    write("PASS integrated MMU EL0 SVC timer scenario\r\n"); halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
