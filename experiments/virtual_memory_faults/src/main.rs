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
    .global fault_vector_table
fault_vector_table:
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b sync_el1_spx
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
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128
    b unexpected_exception
    .balign 128

    .section .text.fault_handler, "ax"
sync_el1_spx:
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
    cmp x1, #0x25
    b.ne unexpected_exception
    tbz x0, #25, unexpected_exception
    tbz x0, #6, unexpected_exception
    tbnz x0, #7, unexpected_exception
    tbnz x0, #10, unexpected_exception
    and x1, x0, #0x3f
    cmp x1, #0x0f
    b.ne unexpected_exception
    adrp x1, FAULT_ESR
    add x1, x1, :lo12:FAULT_ESR
    str x0, [x1]
    mrs x0, FAR_EL1
    adrp x1, RO_PROBE
    add x1, x1, :lo12:RO_PROBE
    cmp x0, x1
    b.ne unexpected_exception
    adrp x1, FAULT_FAR
    add x1, x1, :lo12:FAULT_FAR
    str x0, [x1]
    mrs x0, ELR_EL1
    adrp x1, faulting_write_site
    add x1, x1, :lo12:faulting_write_site
    cmp x0, x1
    b.ne unexpected_exception
    adrp x1, FAULT_PC
    add x1, x1, :lo12:FAULT_PC
    str x0, [x1]
    add x0, x0, #4
    adrp x1, RESUME_PC
    add x1, x1, :lo12:RESUME_PC
    str x0, [x1]
    msr ELR_EL1, x0
    adrp x1, FAULT_COUNT
    add x1, x1, :lo12:FAULT_COUNT
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

    .global faulting_write
faulting_write:
    .global faulting_write_site
faulting_write_site:
    str x1, [x0]
    ret
"#);

const PAGE: usize = 4096;
const ENTRIES: usize = 512;
const RAM_REGION: usize = 0x4000_0000;
const REGION_SIZE: usize = 2 * 1024 * 1024;
const UART: usize = 0x0900_0000;
const UXN: u64 = 1 << 54;
const PXN: u64 = 1 << 53;
const AF: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 3 << 8;
const EL1_READONLY: u64 = 1 << 7;
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
static mut L3_RAM: Table = Table([0; ENTRIES]);
static mut PROBE: u64 = 0;
#[repr(C, align(4096))]
struct ReadOnlyProbe([u64; ENTRIES]);
#[no_mangle]
#[link_section = ".roprobe"]
static mut RO_PROBE: ReadOnlyProbe = ReadOnlyProbe([EXPECTED; ENTRIES]);
#[no_mangle]
static mut FAULT_ESR: u64 = 0;
#[no_mangle]
static mut FAULT_FAR: u64 = 0;
#[no_mangle]
static mut FAULT_PC: u64 = 0;
#[no_mangle]
static mut RESUME_PC: u64 = 0;
#[no_mangle]
static mut FAULT_COUNT: u64 = 0;
extern "C" {
    static __image_start: u8;
    static __image_end: u8;
    static __text_end: u8;
    static __rodata_end: u8;
    static fault_vector_table: u8;
    static faulting_write_site: u8;
    fn faulting_write(address: *mut u64, value: u64);
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
fn mapped_identity(address: usize) -> bool {
    let par=query(address);
    par & 1 == 0 && (par & PA_MASK) == ((address as u64) & PA_MASK)
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut el:u64; let mut sctlr:u64; let mut sp:u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el); asm!("mrs {}, SCTLR_EL1", out(reg) sctlr); asm!("mov {}, sp", out(reg) sp); }
    let start=addr_of!(__image_start) as usize;
    let end=addr_of!(__image_end) as usize;
    let text_end=addr_of!(__text_end) as usize;
    let ro_end=addr_of!(__rodata_end) as usize;
    let data_start=ro_end;
    let vector=addr_of!(fault_vector_table) as usize;
    let ro_probe=addr_of_mut!(RO_PROBE) as usize;
    check(el == 4 && sctlr & ((1<<0)|(1<<2)|(1<<12)|(1<<25)) == 0 && sp & 15 == 0, "unexpected boot state");
    check(start % PAGE == 0 && end % PAGE == 0 && start >= RAM_REGION && end <= RAM_REGION + REGION_SIZE && start < end, "image range");
    check(start < text_end && text_end <= ro_end && ro_end < end && text_end % PAGE == 0 && ro_end % PAGE == 0, "section boundaries");
    check(data_start < end && sp as usize > data_start && (sp as usize) <= end, "stack bounds");
    check(vector % 2048 == 0 && vector >= start && vector.checked_add(2048).is_some_and(|n| n <= text_end), "vector table bounds");
    check(ro_probe % PAGE == 0 && ro_probe >= text_end && ro_probe.checked_add(PAGE).is_some_and(|n| n <= ro_end), "read-only probe bounds");
    unsafe { asm!("msr VBAR_EL1, {}", "isb", in(reg) vector); }
    let mut vbar:u64;
    unsafe { asm!("mrs {}, VBAR_EL1", out(reg) vbar); }
    check(vbar == vector as u64, "VBAR readback");
    let l0=addr_of_mut!(L0); let l1=addr_of_mut!(L1);
    let l2low=addr_of_mut!(L2_LOW); let l2ram=addr_of_mut!(L2_RAM);
    let l3uart=addr_of_mut!(L3_UART); let l3ram=addr_of_mut!(L3_RAM);
    let a0=table_addr(l0,start,end); let a1=table_addr(l1,start,end);
    let a2low=table_addr(l2low,start,end); let a2ram=table_addr(l2ram,start,end);
    let a3uart=table_addr(l3uart,start,end); let a3ram=table_addr(l3ram,start,end);
    // All tables start zeroed in BSS. Entries outside these two L3 regions remain invalid.
    unsafe {
        set(l0,0,a1|TABLE_OR_PAGE);
        set(l1,0,a2low|TABLE_OR_PAGE);
        set(l1,1,a2ram|TABLE_OR_PAGE);
        set(l2low,(UART >> 21)&511,a3uart|TABLE_OR_PAGE);
        set(l2ram,(RAM_REGION >> 21)&511,a3ram|TABLE_OR_PAGE);
        set(l3uart,(UART >> 12)&511,(UART as u64)|AF|PXN|UXN|TABLE_OR_PAGE);
        for address in (start..end).step_by(PAGE) {
            let mut flags=if address < text_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|UXN }
                else if address < ro_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|PXN|UXN }
                else { AF|INNER_SHAREABLE|NORMAL_INDEX|PXN|UXN };
            // Negative control: only the synthetic test page becomes writable.
            if cfg!(feature = "writable-control") && address == ro_probe { flags &= !EL1_READONLY; }
            set(l3ram,(address >> 12)&511,(address as u64)|flags|TABLE_OR_PAGE);
        }
    }
    let addresses=[a0,a1,a2low,a2ram,a3uart,a3ram];
    for i in 0..addresses.len() {
        for j in 0..i { check(addresses[i] != addresses[j], "overlapping tables"); }
    }
    // The configured data and device descriptors are marked execute-never;
    // this reads the entries, not a deliberate attempt to execute those pages.
    let data_entry=unsafe { l3ram.cast::<u64>().add((data_start >> 12)&511).read_volatile() };
    let uart_entry=unsafe { l3uart.cast::<u64>().add((UART >> 12)&511).read_volatile() };
    check(data_entry & (PXN|UXN) == (PXN|UXN) && uart_entry & (PXN|UXN) == (PXN|UXN), "execute-never descriptors");
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
    check(query(start-PAGE)&1 != 0 && query(end)&1 != 0 && query(UART+PAGE)&1 != 0, "neighbor unexpectedly mapped");
    check(query_write(start)&1 != 0 && query_write(probe as usize)&1 == 0
        && query_user_read(probe as usize)&1 != 0 && query_user_read(UART)&1 != 0, "translation access permissions");
    write("MMU enabled=yes translation=identity text=ro-exec data=rw-xn uart=device-xn\r\n");
    write("CHECK probe=read-write stack=translated uart=translated neighbor-pages=unmapped\r\n");
    write("CHECK text-write=denied data-write=allowed el0-reads=denied xn=descriptor-bits\r\n");
    check(mapped_identity(ro_probe), "probe translation");
    if cfg!(feature = "writable-control") {
        check(query_write(ro_probe)&1 == 0, "control probe not writable");
    } else {
        check(query_write(ro_probe)&1 != 0, "probe not read-only");
    }
    let target = addr_of_mut!(RO_PROBE).cast::<u64>();
    for i in 0..ENTRIES {
        check(unsafe { read_volatile(target.add(i)) } == EXPECTED, "probe initial page");
    }
    // This explicit assembly store is expected to trap. The handler validates
    // syndrome, FAR and ELR before skipping only that instruction.
    unsafe { faulting_write(target, 0xfedc_ba98_7654_3210); }
    let esr=unsafe { read_volatile(addr_of!(FAULT_ESR)) };
    let far=unsafe { read_volatile(addr_of!(FAULT_FAR)) };
    let pc=unsafe { read_volatile(addr_of!(FAULT_PC)) };
    let resume=unsafe { read_volatile(addr_of!(RESUME_PC)) };
    let count=unsafe { read_volatile(addr_of!(FAULT_COUNT)) };
    if cfg!(feature = "writable-control") {
        check(count == 0 && unsafe { read_volatile(target) } == 0xfedc_ba98_7654_3210, "writable control store");
        for i in 1..ENTRIES {
            check(unsafe { read_volatile(target.add(i)) } == EXPECTED, "writable control neighbor");
        }
        write("CONTROL writable_store=completed faults=0 other_words=unchanged\r\n");
        write("PASS writable permission control\r\n");
        halt();
    }
    check(count == 1 && esr >> 26 == 0x25 && esr & (1<<25) != 0 && esr & (1<<6) != 0 && esr & (1<<7) == 0 && esr & (1<<10) == 0 && esr & 0x3f == 0x0f, "fault syndrome or count");
    check(far == ro_probe as u64 && pc == addr_of!(faulting_write_site) as u64 && pc.checked_add(4) == Some(resume), "fault addresses");
    for i in 0..ENTRIES {
        check(unsafe { read_volatile(target.add(i)) } == EXPECTED, "read-only page changed");
    }
    write("FAULT ec=0x25 wnr=1 dfsc=0x0f far=probe elr=faulting-store count=1\r\n");
    write("RECOVERY elr=next-instruction page=unchanged words=512\r\n");
    write("PASS controlled EL1 write permission fault\r\n");
    halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
