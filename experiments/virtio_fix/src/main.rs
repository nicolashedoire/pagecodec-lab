#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};
use zerocopy::{FromBytes, Immutable, IntoBytes};
use virtio_drivers::{
    device::rng::VirtIORng,
    transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceStatus, DeviceType, InterruptStatus, Transport},
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

#[derive(Default)]
struct MockState { status: DeviceStatus, page_size_calls: usize, queue_calls: usize, driver_ok_calls: usize, needs_reset: bool }
struct Rejecting<'a>(&'a mut MockState);
impl Transport for Rejecting<'_> {
    fn device_type(&self) -> DeviceType { DeviceType::EntropySource }
    fn read_device_features(&mut self) -> u64 { 1u64 << 32 }
    fn write_driver_features(&mut self, _features: u64) {}
    fn max_queue_size(&mut self, _queue: u16) -> u32 { self.0.queue_calls += 1; 8 }
    fn notify(&mut self, _queue: u16) {}
    fn get_status(&self) -> DeviceStatus { self.0.status }
    fn set_status(&mut self, status: DeviceStatus) {
        if status.contains(DeviceStatus::DRIVER_OK) { self.0.driver_ok_calls += 1; }
        self.0.status = if self.0.needs_reset && status.contains(DeviceStatus::FEATURES_OK) {
            status | DeviceStatus::DEVICE_NEEDS_RESET
        } else { status & !DeviceStatus::FEATURES_OK };
    }
    fn set_guest_page_size(&mut self, _size: u32) { self.0.page_size_calls += 1; }
    fn requires_legacy_layout(&self) -> bool { false }
    fn queue_set(&mut self, _q: u16, _s: u32, _d: PhysAddr, _a: PhysAddr, _u: PhysAddr) { self.0.queue_calls += 1; }
    fn queue_unset(&mut self, _q: u16) { self.0.queue_calls += 1; }
    fn queue_used(&mut self, _q: u16) -> bool { self.0.queue_calls += 1; false }
    fn ack_interrupt(&mut self) -> InterruptStatus { InterruptStatus::empty() }
    fn read_config_generation(&self) -> u32 { 0 }
    fn read_config_space<T: FromBytes + IntoBytes>(&self, _offset: usize) -> virtio_drivers::Result<T> { Err(virtio_drivers::Error::Unsupported) }
    fn write_config_space<T: IntoBytes + Immutable>(&mut self, _offset: usize, _value: T) -> virtio_drivers::Result<()> { Err(virtio_drivers::Error::Unsupported) }
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut state = MockState::default();
    let before_dma = DMA_NEXT.load(Ordering::Relaxed);
    let refused = matches!(VirtIORng::<VmHal, _>::new(Rejecting(&mut state)), Err(virtio_drivers::Error::FeatureNegotiationFailed));
    if !refused || !state.status.contains(DeviceStatus::FAILED) || state.queue_calls != 0 || state.page_size_calls != 0 || state.driver_ok_calls != 0 || DMA_NEXT.load(Ordering::Relaxed) != before_dma {
        write("FAIL constructor did not propagate simulated refusal\r\n"); halt();
    }
    write("MOCK rng_constructor=feature_error failed=yes queue_calls=0 dma=0 driver_ok=0\r\n");
    let mut reset_state = MockState { needs_reset: true, ..MockState::default() };
    let reset_refused = matches!(VirtIORng::<VmHal, _>::new(Rejecting(&mut reset_state)), Err(virtio_drivers::Error::FeatureNegotiationFailed));
    if !reset_refused || !reset_state.status.contains(DeviceStatus::FAILED) || reset_state.queue_calls != 0 || reset_state.driver_ok_calls != 0 {
        write("FAIL needs-reset did not stop init\r\n"); halt();
    }
    write("MOCK rng_constructor=needs_reset_error queue_calls=0 driver_ok=0\r\n");
    for i in 0..MMIO_SLOTS {
        let ptr = NonNull::new((MMIO_BASE + i * MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        let Ok(transport) = (unsafe { MmioTransport::new(ptr, MMIO_STRIDE) }) else { continue };
        if transport.device_type() != DeviceType::EntropySource { continue; }
        let legacy = transport.requires_legacy_layout();
        let Ok(mut rng) = VirtIORng::<VmHal, _>::new(transport) else { write("FAIL QEMU rng init\r\n"); halt() };
        let mut bytes = [0u8; 32];
        if !matches!(rng.request_entropy(&mut bytes), Ok(32)) || !bytes.iter().enumerate().all(|(i, &b)| b == i as u8) {
            write("FAIL QEMU rng bytes\r\n"); halt();
        }
        if legacy { write("QEMU legacy rng_init=ok deterministic_bytes=32\r\n"); }
        else { write("QEMU modern rng_init=ok deterministic_bytes=32\r\n"); }
        write("PASS patched virtio rng scenario\r\n"); halt();
    }
    write("FAIL no QEMU rng\r\n"); halt();
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
