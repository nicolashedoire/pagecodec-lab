#![no_std]
#![no_main]

extern crate alloc;
use alloc::string::String;

use actor_core_sketch::{ActorId, ActorKind, Registry, SpaceId};
use system_core_sketch::{Document, Origin};
use workspace_bridge_sketch::Bridge;
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

// Versioned single-sector synthetic record. FNV-1a detects accidental corruption,
// not adversarial modification; the fixture has no authentication or transactions.
const MAGIC: &[u8;8] = b"WSPERS01";
const INIT: &[u8;8] = b"VMINIT01";
const LOAD: &[u8;8] = b"VMLOAD01";
fn checksum(bytes: &[u8]) -> u32 {
    let mut h=2166136261u32;
    for b in bytes { h=(h ^ u32::from(*b)).wrapping_mul(16777619); }
    h
}
fn setup(title: &str, status: &str) -> Option<(String,String)> {
    let human=ActorId::new("human").unwrap(); let ai=ActorId::new("assistant").unwrap();
    let hs=SpaceId::new("human-space").unwrap(); let ais=SpaceId::new("assistant-space").unwrap();
    let mut r=Registry::new();
    r.create_space(hs.clone()).unwrap(); r.create_space(ais.clone()).unwrap();
    r.create_actor(human.clone(),ActorKind::Human).unwrap(); r.create_actor(ai.clone(),ActorKind::Assistant).unwrap();
    r.assign(&human,&hs).unwrap(); r.assign(&ai,&ais).unwrap();
    let mut base=Document::new(); base.insert("title".into(),"base".into());
    let mut bridge=Bridge::new(r,human.clone(),hs.clone(),ai.clone(),ais.clone(),base).unwrap();
    bridge.edit(&human,&hs,"title",Some(title)).unwrap(); bridge.edit(&ai,&ais,"status",Some(status)).unwrap();
    let result=bridge.merge();
    let valid=result.conflicts.is_empty()
        && result.fields.get("title").is_some_and(|f|f.value.as_deref()==Some(title)&&f.origin==Origin::Human)
        && result.fields.get("status").is_some_and(|f|f.value.as_deref()==Some(status)&&f.origin==Origin::Assistant)
        && bridge.view(&human,&hs).unwrap().get("status").is_none()
        && bridge.view(&ai,&ais).unwrap().get("title").is_some_and(|v|v=="base");
    if !valid { return None }
    // Serialization uses the actual values read from each bridge copy.
    Some((bridge.view(&human,&hs).ok()?.get("title")?.clone(),
          bridge.view(&ai,&ais).ok()?.get("status")?.clone()))
}
fn encode(title: &str, status: &str) -> Option<[u8;512]> {
    if title.is_empty() || status.is_empty() || title.len()>64 || status.len()>64 { return None }
    let mut b=[0u8;512]; b[..8].copy_from_slice(MAGIC); b[8]=title.len() as u8; b[9]=status.len() as u8;
    b[10..10+title.len()].copy_from_slice(title.as_bytes());
    b[10+title.len()..10+title.len()+status.len()].copy_from_slice(status.as_bytes());
    let sum=checksum(&b[..508]).to_le_bytes(); b[508..].copy_from_slice(&sum); Some(b)
}
fn decode(b: &[u8;512]) -> Option<(&str,&str)> {
    if &b[..8]!=MAGIC { return None }
    let a=b[8] as usize; let c=b[9] as usize;
    if a==0 || c==0 || a>64 || c>64 { return None }
    if checksum(&b[..508]) != u32::from_le_bytes([b[508],b[509],b[510],b[511]]) { return None }
    if b[10+a+c..508].iter().any(|&x|x!=0) { return None }
    Some((core::str::from_utf8(&b[10..10+a]).ok()?, core::str::from_utf8(&b[10+a..10+a+c]).ok()?))
}
#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    for slot in 0..MMIO_SLOTS {
        let ptr=NonNull::new((MMIO_BASE+slot*MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        let Ok(transport)=(unsafe { MmioTransport::new(ptr,MMIO_STRIDE) }) else {continue};
        if transport.device_type()!=DeviceType::Block {continue}
        let Ok(mut disk)=VirtIOBlk::<VmHal,_>::new(transport) else {write("FAIL init\r\n"); halt()};
        if disk.capacity()!=2048 {write("FAIL capacity\r\n"); halt()}
        let mut mode=[0u8;512]; let mut record=[0u8;512];
        if disk.read_blocks(0,&mut mode).is_err() || disk.read_blocks(4,&mut record).is_err() {write("FAIL read\r\n"); halt()}
        if mode[8..].iter().any(|&x|x!=0) {write("REJECT invalid mode\r\n"); halt()}
        if &mode[..8]==INIT {
            if record.iter().any(|&b|b!=0) {write("REJECT nonempty init\r\n"); halt()}
            let title="human-version"; let status="assistant-ready";
            let Some((stored_title,stored_status))=setup(title,status) else {write("FAIL bridge\r\n"); halt()};
            let encoded=encode(&stored_title,&stored_status).unwrap();
            if disk.write_blocks(4,&encoded).is_err() || disk.flush().is_err() {write("FAIL write or flush\r\n"); halt()}
            let mut actual=[0u8;512];
            if disk.read_blocks(4,&mut actual).is_err() || actual!=encoded {write("FAIL readback\r\n"); halt()}
            write("EDIT human title=human-version; assistant status=assistant-ready\r\nWRITE sector=4 bytes=512 flush=ok readback=exact\r\nPASS save synthetic bridge record\r\n"); halt()
        }
        if &mode[..8]==LOAD {
            let Some((title,status))=decode(&record) else {write("REJECT malformed record\r\n"); halt()};
            if setup(title,status).is_none() {write("FAIL restored bridge\r\n"); halt()}
            if title!="human-version" || status!="assistant-ready" {write("REJECT unexpected fixture values\r\n"); halt()}
            write("READ sector=4 bytes=512 checksum=ok\r\nRESTORE human title=human-version origin=human; assistant status=assistant-ready origin=assistant copies=separate\r\nPASS restore synthetic bridge record\r\n"); halt()
        }
        write("REJECT invalid mode\r\n"); halt()
    }
    write("FAIL no virtio block device\r\n"); halt()
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
