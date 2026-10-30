#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};
use virtio_drivers::{
    device::blk::VirtIOBlk,
    transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceStatus, DeviceType, InterruptStatus, Transport},
    BufferDirection, Hal, PhysAddr,
};
use zerocopy::{FromBytes, Immutable, IntoBytes};

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

// In-guest synthetic transport: it clears FEATURES_OK to model refusal.
#[derive(Default)]
struct MockState { status: DeviceStatus, config_calls: Cell<usize>, queue_calls: usize, driver_ok_calls: usize }
struct Rejecting<'a>(&'a mut MockState);
impl Transport for Rejecting<'_> {
    fn device_type(&self) -> DeviceType { DeviceType::Block }
    fn read_device_features(&mut self) -> u64 { 1u64 << 32 }
    fn write_driver_features(&mut self, _: u64) {}
    fn max_queue_size(&mut self, _: u16) -> u32 { self.0.queue_calls += 1; 8 }
    fn notify(&mut self, _: u16) {}
    fn get_status(&self) -> DeviceStatus { self.0.status }
    fn set_status(&mut self, status: DeviceStatus) {
        if status.contains(DeviceStatus::DRIVER_OK) { self.0.driver_ok_calls += 1; }
        self.0.status = status & !DeviceStatus::FEATURES_OK;
    }
    fn set_guest_page_size(&mut self, _: u32) {}
    fn requires_legacy_layout(&self) -> bool { false }
    fn queue_set(&mut self, _: u16, _: u32, _: PhysAddr, _: PhysAddr, _: PhysAddr) { self.0.queue_calls += 1; }
    fn queue_unset(&mut self, _: u16) { self.0.queue_calls += 1; }
    fn queue_used(&mut self, _: u16) -> bool { self.0.queue_calls += 1; false }
    fn ack_interrupt(&mut self) -> InterruptStatus { InterruptStatus::empty() }
    fn read_config_generation(&self) -> u32 { self.0.config_calls.set(self.0.config_calls.get() + 1); 0 }
    fn read_config_space<T: FromBytes + IntoBytes>(&self, _: usize) -> virtio_drivers::Result<T> {
        self.0.config_calls.set(self.0.config_calls.get() + 1); Err(virtio_drivers::Error::Unsupported)
    }
    fn write_config_space<T: IntoBytes + Immutable>(&mut self, _: usize, _: T) -> virtio_drivers::Result<()> {
        self.0.config_calls.set(self.0.config_calls.get() + 1); Err(virtio_drivers::Error::Unsupported)
    }
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut mock = MockState::default();
    let before_dma = DMA_NEXT.load(Ordering::Relaxed);
    let refused = matches!(VirtIOBlk::<VmHal, _>::new(Rejecting(&mut mock)), Err(virtio_drivers::Error::FeatureNegotiationFailed));
    if !refused || !mock.status.contains(DeviceStatus::FAILED) || mock.config_calls.get() != 0 || mock.queue_calls != 0 || mock.driver_ok_calls != 0 || DMA_NEXT.load(Ordering::Relaxed) != before_dma {
        write("FAIL simulated features refusal\r\n"); halt();
    }
    write("CHECK simulated_features_refusal=error no_config_no_queue_no_dma\r\n");
    for slot in 0..MMIO_SLOTS {
        let ptr = NonNull::new((MMIO_BASE + slot * MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        let Ok(transport) = (unsafe { MmioTransport::new(ptr, MMIO_STRIDE) }) else { continue };
        if transport.device_type() != DeviceType::Block { continue; }
        let Ok(mut disk) = VirtIOBlk::<VmHal, _>::new(transport) else { write("FAIL init\r\n"); halt() };
        write("DEVICE capacity_sectors="); write_decimal(disk.capacity() as usize); write("\r\n");
        // The harness creates a new 1 MiB all-zero fixture for this VM only.
        if disk.capacity() != 2048 { write("FAIL fixture size\r\n"); halt() }
        let mut original = [0u8; 1024];
        if disk.read_blocks(4, &mut original).is_err() { write("FAIL initial read\r\n"); halt() }
        if original.iter().any(|&b| b != 0) { write("FAIL fixture not zero\r\n"); halt() }
        write("READ sectors=4..5 initial=zeros\r\n");
        let mut expected = [0u8; 1024];
        for (i, byte) in expected.iter_mut().enumerate() { *byte = ((i * 37 + 11) % 256) as u8; }
        if disk.write_blocks(4, &expected).is_err() { write("FAIL write\r\n"); halt() }
        write("WRITE sectors=4..5 bytes=1024 pattern=(offset*37+11)%256\r\n");
        if disk.flush().is_err() { write("FAIL flush\r\n"); halt() }
        write("FLUSH ok\r\n");
        let mut actual = [0u8; 1024];
        if disk.read_blocks(4, &mut actual).is_err() || actual != expected { write("FAIL readback\r\n"); halt() }
        let mut neighbor = [0u8; 512];
        if disk.read_blocks(3, &mut neighbor).is_err() || neighbor.iter().any(|&b| b != 0) {
            write("FAIL lower neighbor\r\n"); halt();
        }
        if disk.read_blocks(6, &mut neighbor).is_err() || neighbor.iter().any(|&b| b != 0) {
            write("FAIL upper neighbor\r\n"); halt();
        }
        write("READBACK bytes=1024 exact=yes neighboring_sectors=zero\r\n");
        write("PASS synthetic virtio block read write flush\r\n");
        halt();
    }
    write("FAIL no virtio block device\r\n"); halt()
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
