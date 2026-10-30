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
use local_model_loader::{parse, sha256, Input, Error};

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
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }

#[no_mangle]
pub extern "C" fn rust_main()->! {
    // An independent, standard SHA-256 test vector inside the guest.
    if sha256(b"abc") != [0xba,0x78,0x16,0xbf,0x8f,0x01,0xcf,0xea,0x41,0x41,0x40,0xde,0x5d,0xae,0x22,0x23,0xb0,0x03,0x61,0xa3,0x96,0x17,0x7a,0x9c,0xb4,0x10,0xff,0x61,0xf2,0x00,0x15,0xad] {write("FAIL sha vector\r\n");halt()}
    let mut vectors=[0u8;512];for (i,v) in vectors.iter_mut().enumerate(){*v=(i%251) as u8;}
    let checks=[(0usize,[0xe3,0xb0,0xc4,0x42,0x98,0xfc,0x1c,0x14,0x9a,0xfb,0xf4,0xc8,0x99,0x6f,0xb9,0x24,0x27,0xae,0x41,0xe4,0x64,0x9b,0x93,0x4c,0xa4,0x95,0x99,0x1b,0x78,0x52,0xb8,0x55]),(55usize,[0x46,0x3e,0xb2,0x8e,0x72,0xf8,0x2e,0xa,0x96,0xc0,0xa4,0xcc,0x53,0x69,0xc,0x57,0x12,0x81,0x13,0x1f,0x67,0x2a,0xa2,0x29,0xe0,0xd4,0x5a,0xe5,0x9b,0x59,0x8b,0x59]),(56usize,[0xda,0x2a,0xe4,0xd6,0xb3,0x67,0x48,0xf2,0xa3,0x18,0xf2,0x3e,0x7a,0xb1,0xdf,0xdf,0x45,0xac,0xdc,0x9d,0x4,0x9b,0xd8,0xe,0x59,0xde,0x82,0xa6,0x8,0x95,0xf5,0x62]),(64usize,[0xfd,0xea,0xb9,0xac,0xf3,0x71,0x3,0x62,0xbd,0x26,0x58,0xcd,0xc9,0xa2,0x9e,0x8f,0x9c,0x75,0x7f,0xcf,0x98,0x11,0x60,0x3a,0x8c,0x44,0x7c,0xd1,0xd9,0x15,0x11,0x8]),(512usize,[0xd8,0x6e,0x38,0x62,0x78,0xa7,0x17,0x82,0xa2,0x83,0xf9,0x6a,0xae,0x4f,0x4e,0x74,0x37,0x47,0x1a,0xbe,0xf7,0x11,0x36,0xbd,0x28,0x11,0xf9,0x82,0x45,0x48,0x8d,0x89])];
    for (n,digest) in checks {if sha256(&vectors[..n])!=digest {write("FAIL sha boundary\r\n");halt()}}
    for slot in 0..MMIO_SLOTS {
        let ptr=NonNull::new((MMIO_BASE+slot*MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        let Ok(transport)=(unsafe {MmioTransport::new(ptr,MMIO_STRIDE)}) else {continue};
        if transport.device_type()!=DeviceType::Block {continue}
        let Ok(mut disk)=VirtIOBlk::<VmHal,_>::new(transport) else {write("FAIL init\r\n");halt()};
        if disk.capacity()!=2048 {write("FAIL capacity\r\n");halt()}
        let mut sector=[0u8;512];
        if disk.read_blocks(0,&mut sector).is_err(){write("FAIL read\r\n");halt()}
        let model=match parse(&sector) {
            Ok(m)=>m,
            Err(e)=>{write(match e {Error::Magic=>"REJECT magic\r\n",Error::Version=>"REJECT version\r\n",Error::Overflow=>"REJECT overflow\r\n",Error::Shape=>"REJECT shape\r\n",Error::Length=>"REJECT length\r\n",Error::Hash=>"REJECT hash\r\n",Error::Padding=>"REJECT padding\r\n"});halt()}
        };
        // Four separate observations, expected values independently computed in Python.
        let cases=[([0,0,0],[0,20,0,30],[-40,30]),([100,0,0],[250,0,100,0],[610,-70]),([0,100,0],[0,420,0,0],[-410,400]),([100,100,100],[150,220,400,30],[460,880])];
        for (v,hidden,scores) in cases {
            let Some(input)=Input::new(v) else {write("FAIL input\r\n");halt()};
            let Some(result)=model.infer(input) else {write("FAIL arithmetic\r\n");halt()};
            if result.hidden!=hidden || result.scores!=scores {write("FAIL reference\r\n");halt()}
        }
        if Input::new([101,0,0]).is_some(){write("FAIL bounds\r\n");halt()}
        write("CHECK sha256=abc,boundaries shape=3x4x2 payload=32 input=bounded\r\n");
        write("CHECK cases=4 hidden_and_scores=reference guest_writes=none\r\n");
        write("SCORES zero=-40,30 cpu=610,-70 load=-410,400 all=460,880\r\n");
        write("PASS synthetic model load and inference\r\n");halt();
    }
    write("FAIL no block device\r\n");halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>)->! {write("FAIL kernel panic\r\n");halt()}
