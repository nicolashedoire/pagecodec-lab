#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};
use virtio_drivers::{
    device::blk::VirtIOBlk,
    transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceType, Transport},
    BufferDirection, Hal, PhysAddr,
};
use io_journal_sketch::{Actor, Category, DecodeError, Full, Journal, Outcome, CAPACITY};

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
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }

fn expected_journal()->Journal {
    let mut j=Journal::new();
    for i in 0..CAPACITY {
        let category=[Category::DiskRead,Category::DiskWrite,Category::Display,Category::Input][i%4];
        let actor=[Actor::Human,Actor::Assistant,Actor::System][i%3];
        let outcome=[Outcome::Ok,Outcome::Rejected,Outcome::Failed][i%3];
        j.append(category,actor,outcome).unwrap();
    }
    j
}

#[no_mangle]
pub extern "C" fn rust_main()->! {
    for slot in 0..MMIO_SLOTS {
        let ptr=NonNull::new((MMIO_BASE+slot*MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        let Ok(transport)=(unsafe {MmioTransport::new(ptr,MMIO_STRIDE)}) else {continue};
        if transport.device_type()!=DeviceType::Block {continue}
        let Ok(mut disk)=VirtIOBlk::<VmHal,_>::new(transport) else {write("FAIL init\r\n");halt()};
        if disk.capacity()!=2048 {write("FAIL capacity\r\n");halt()}
        let mut mode=[0u8;512];let mut sector=[0u8;512];
        if disk.read_blocks(0,&mut mode).is_err() || disk.read_blocks(4,&mut sector).is_err() {write("FAIL read\r\n");halt()}
        if mode[10..].iter().any(|&b|b!=0) {write("REJECT mode\r\n");halt()}
        let length=u16::from_le_bytes([mode[8],mode[9]]) as usize;
        if &mode[..8]==b"IOINIT01" {
            if length!=0 || sector.iter().any(|&b|b!=0) {write("REJECT nonempty init\r\n");halt()}
            let mut journal=expected_journal();let before=journal.encode();
            if journal.append(Category::Input,Actor::System,Outcome::Ok)!=Err(Full) || journal.encode()!=before {write("FAIL capacity invariant\r\n");halt()}
            if Journal::decode(&before)!=Ok(journal) || Journal::decode(&before[..511])!=Err(DecodeError::Length) {write("FAIL encode or length\r\n");halt()}
            if disk.write_blocks(4,&before).is_err() || disk.flush().is_err() {write("FAIL write or flush\r\n");halt()}
            let mut actual=[0u8;512];if disk.read_blocks(4,&mut actual).is_err() || actual!=before {write("FAIL readback\r\n");halt()}
            write("APPEND events=16 sequences=1..16 categories=4 actors=3 outcomes=3 payload=none\r\n");
            write("CHECK full=refused prior_records=unchanged record_bytes=512 flush_call=ok readback=exact\r\n");
            write("PASS save synthetic journal\r\n");halt();
        }
        if &mode[..8]==b"IOLOAD01" {
            if length>512 {write("REJECT length\r\n");halt()}
            let decoded=match Journal::decode(&sector[..length]) {
                Ok(j)=>j,
                Err(DecodeError::Length)=>{write("REJECT length\r\n");halt()},
                Err(DecodeError::Checksum)=>{write("REJECT checksum\r\n");halt()},
                Err(DecodeError::Count)=>{write("REJECT count\r\n");halt()},
                Err(_)=>{write("REJECT malformed\r\n");halt()},
            };
            if decoded!=expected_journal() {write("REJECT unexpected fixture\r\n");halt()}
            write("REPLAY events=16 sequences=1..16 categories=4 actors=3 outcomes=3 io_actions=none\r\n");
            write("PASS replay synthetic journal\r\n");halt();
        }
        write("REJECT mode\r\n");halt();
    }
    write("FAIL no block device\r\n");halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>)->! {write("FAIL kernel panic\r\n");halt()}
