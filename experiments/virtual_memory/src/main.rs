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
extern "C" {
    static __image_start: u8;
    static __image_end: u8;
    static __text_end: u8;
    static __rodata_end: u8;
}

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (UART as *const u32).add(6).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (UART as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, message: &str) { if !ok { write("FAIL "); write(message); write("\r\n"); halt() } }
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
    check(el == 4 && sctlr & ((1<<0)|(1<<2)|(1<<12)|(1<<25)) == 0 && sp & 15 == 0, "unexpected boot state");
    check(start % PAGE == 0 && end % PAGE == 0 && start >= RAM_REGION && end <= RAM_REGION + REGION_SIZE && start < end, "image range");
    check(start < text_end && text_end <= ro_end && ro_end < end && text_end % PAGE == 0 && ro_end % PAGE == 0, "section boundaries");
    check(data_start < end && sp as usize > data_start && (sp as usize) <= end, "stack bounds");
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
            let flags=if address < text_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|UXN }
                else if address < ro_end { AF|INNER_SHAREABLE|NORMAL_INDEX|EL1_READONLY|PXN|UXN }
                else { AF|INNER_SHAREABLE|NORMAL_INDEX|PXN|UXN };
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
    write("PASS bounded EL1 virtual memory scenario\r\n");
    halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
