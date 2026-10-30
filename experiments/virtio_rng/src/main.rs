#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};
use system_core_sketch::{Actor, Document, Origin, Workspaces};
use virtio_drivers::{
    device::rng::VirtIORng,
    transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceType, Transport},
    BufferDirection, Hal, PhysAddr,
};

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
1:
    cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:
    ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
3:
    wfe
    b 3b
"#);

const PAGE: usize = 4096;
const HEAP_SIZE: usize = 64 * 1024;
const DMA_SIZE: usize = 64 * 1024;
const MMIO_BASE: usize = 0x0a00_0000;
const MMIO_STRIDE: usize = 0x200;
const MMIO_SLOTS: usize = 32;
const RAM_BASE: usize = 0x4000_0000;
const RAM_END: usize = RAM_BASE + 128 * 1024 * 1024;

fn in_guest_ram(addr: usize, len: usize) -> bool {
    len > 0 && addr >= RAM_BASE && addr.checked_add(len).is_some_and(|end| end <= RAM_END)
}

#[repr(align(4096))]
struct PageAligned<const N: usize>([u8; N]);
static mut HEAP: PageAligned<HEAP_SIZE> = PageAligned([0; HEAP_SIZE]);
static mut DMA: PageAligned<DMA_SIZE> = PageAligned([0; DMA_SIZE]);
static HEAP_NEXT: AtomicUsize = AtomicUsize::new(0);
static DMA_NEXT: AtomicUsize = AtomicUsize::new(0);

struct Bump;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let base = addr_of_mut!(HEAP) as *mut u8 as usize;
        let mut current = HEAP_NEXT.load(Ordering::Relaxed);
        loop {
            let Some(start) = base.checked_add(current)
                .and_then(|n| n.checked_add(align - 1))
                .map(|n| (n & !(align - 1)) - base) else { return core::ptr::null_mut() };
            let Some(end) = start.checked_add(layout.size()) else { return core::ptr::null_mut() };
            if end > HEAP_SIZE { return core::ptr::null_mut(); }
            match HEAP_NEXT.compare_exchange_weak(current, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return (addr_of_mut!(HEAP) as *mut u8).add(start),
                Err(n) => current = n,
            }
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) { /* one-shot test, never reuse */ }
}
#[global_allocator]
static ALLOC: Bump = Bump;

// The VM is single-CPU with the MMU and caches disabled. Guest physical and
// virtual addresses are identical; this HAL is NOT suitable for real DMA.
struct VmHal;
unsafe impl Hal for VmHal {
    fn dma_alloc(pages: usize, _: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let size = pages.checked_mul(PAGE).expect("DMA overflow");
        let offset = DMA_NEXT.fetch_add(size, Ordering::Relaxed);
        assert!(size != 0 && offset <= DMA_SIZE && size <= DMA_SIZE - offset);
        // SAFETY: each allocation is page-aligned, non-overlapping and zeroed;
        // all pointers address reserved guest RAM for this one-shot test.
        let p = unsafe { (addr_of_mut!(DMA) as *mut u8).add(offset) };
        assert!(in_guest_ram(p as usize, size));
        (p as PhysAddr, NonNull::new(p).unwrap())
    }
    unsafe fn dma_dealloc(_: PhysAddr, _: NonNull<u8>, _: usize) -> i32 { 0 }
    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, size: usize) -> NonNull<u8> {
        assert!(size > 0 && paddr >= MMIO_BASE as u64 && paddr.checked_add(size as u64).is_some_and(|n| n <= (MMIO_BASE + MMIO_STRIDE * MMIO_SLOTS) as u64));
        NonNull::new(paddr as *mut u8).unwrap()
    }
    unsafe fn share(buffer: NonNull<[u8]>, _: BufferDirection) -> PhysAddr {
        let addr = buffer.as_ptr() as *mut u8 as usize;
        assert!(in_guest_ram(addr, buffer.len()));
        addr as PhysAddr
    }
    unsafe fn unshare(_: PhysAddr, _: NonNull<[u8]>, _: BufferDirection) {}
}

fn put(b: u8) {
    unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }
}
fn write(s: &str) { for b in s.bytes() { put(b); } }
fn write_decimal(mut n: usize) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 { break; }
    }
    for &b in &digits[i..] { put(b); }
}
fn write_hex(bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes { put(HEX[(b >> 4) as usize]); put(HEX[(b & 15) as usize]); }
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }

fn test_workspaces() -> bool {
    write("WORKSPACE_INPUT base.title=base human.title=human assistant.status=assistant\r\n");
    let mut base = Document::new();
    base.insert("title".into(), "base".into());
    let mut copies = Workspaces::from_base(base);
    copies.edit(Actor::Human).insert("title".into(), "human".into());
    copies.edit(Actor::Assistant).insert("status".into(), "assistant".into());
    let result = copies.merge();
    let Some(title) = result.fields.get("title") else { return false };
    let Some(status) = result.fields.get("status") else { return false };
    if !result.conflicts.is_empty() || title.value.as_deref() != Some("human")
        || title.origin != Origin::Human || status.value.as_deref() != Some("assistant")
        || status.origin != Origin::Assistant || result.materialize().is_err() { return false; }
    write("WORKSPACE_CHECK independent=ok human=human assistant=assistant\r\n");

    write("WORKSPACE_INPUT conflict.base=base conflict.human=human conflict.assistant=assistant\r\n");
    let mut base = Document::new();
    base.insert("title".into(), "base".into());
    let mut copies = Workspaces::from_base(base);
    copies.edit(Actor::Human).insert("title".into(), "human".into());
    copies.edit(Actor::Assistant).insert("title".into(), "assistant".into());
    let result = copies.merge();
    if result.conflicts.len() != 1 || result.materialize().is_ok() { return false; }
    let conflict = &result.conflicts[0];
    if conflict.key != "title" || conflict.base.as_deref() != Some("base")
        || conflict.human.as_deref() != Some("human")
        || conflict.assistant.as_deref() != Some("assistant") { return false; }
    write("WORKSPACE_CHECK conflict_preserved=ok materialize_rejected=ok\r\n");
    true
}

fn test_io_merge(bytes: &[u8]) -> bool {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 15) as usize] as char);
    }
    let mut base = Document::new();
    base.insert("title".into(), "base".into());
    let mut copies = Workspaces::from_base(base);
    copies.edit(Actor::Human).insert("title".into(), "human".into());
    copies.edit(Actor::Assistant).insert("device_data".into(), hex.clone());
    let result = copies.merge();
    let Some(title) = result.fields.get("title") else { return false };
    let Some(device_data) = result.fields.get("device_data") else { return false };
    if !result.conflicts.is_empty() || title.value.as_deref() != Some("human")
        || title.origin != Origin::Human || device_data.value.as_deref() != Some(hex.as_str())
        || device_data.origin != Origin::Assistant || result.materialize().is_err() { return false; }
    write("IO_MERGE_CHECK human=human assistant=device_data data_hex=");
    write(&hex);
    write("\r\n");
    true
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let current_el: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) current_el, options(nomem, nostack, preserves_flags)) };
    if current_el != 4 { write("FAIL: expected EL1\r\n"); halt(); }
    let sctlr: u64;
    unsafe { asm!("mrs {}, SCTLR_EL1", out(reg) sctlr, options(nomem, nostack, preserves_flags)) };
    if sctlr & 0b101 != 0 { write("FAIL: MMU or data cache enabled\r\n"); halt(); }
    if !test_workspaces() { write("FAIL: workspace checks\r\n"); halt(); }
    for i in 0..MMIO_SLOTS {
        let addr = MMIO_BASE + i * MMIO_STRIDE;
        let ptr = NonNull::new(addr as *mut VirtIOHeader).unwrap();
        // SAFETY: QEMU virt maps all 32 0x200-byte VirtIO MMIO windows here;
        // each is inspected only once on a single CPU.
        let Ok(transport) = (unsafe { MmioTransport::new(ptr, MMIO_STRIDE) }) else { continue };
        if transport.device_type() != DeviceType::EntropySource { continue; }
        let Ok(mut rng) = VirtIORng::<VmHal, _>::new(transport) else { write("FAIL: RNG init\r\n"); halt() };
        let mut bytes = [0u8; 32];
        let Ok(count) = rng.request_entropy(&mut bytes) else { write("FAIL: RNG request\r\n"); halt() };
        // Deliberately publish these bytes for an explicitly synthetic fixture.
        // Never attach a secret or real entropy source to this lab image.
        write("RECEIVED count=");
        write_decimal(count);
        write(" hex=");
        write_hex(&bytes[..core::cmp::min(count, bytes.len())]);
        write("\r\n");
        if count == 32 && bytes.iter().enumerate().all(|(i, &b)| b == i as u8) {
            if !test_io_merge(&bytes[..count]) { write("FAIL: I/O merge\r\n"); halt(); }
            write("PASS: synthetic bytes via VirtIO RNG\r\n");
        } else { write("FAIL: unexpected RNG data\r\n"); }
        halt();
    }
    write("PASS: no recognized VirtIO entropy device\r\n");
    halt()
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL: kernel panic\r\n"); halt() }
