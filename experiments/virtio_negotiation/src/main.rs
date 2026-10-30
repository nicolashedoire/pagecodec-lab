#![no_std]
#![no_main]
extern crate alloc;

use core::{alloc::{GlobalAlloc, Layout}, arch::{asm, global_asm}, panic::PanicInfo,
           ptr::addr_of_mut, sync::atomic::{AtomicUsize, Ordering}};
use core::ptr::NonNull;
use virtio_drivers::{device::common::Feature, transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceStatus, DeviceType, InterruptStatus, Transport}, PhysAddr};
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
1:  cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:  ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
3:  wfe
    b 3b
"#);

const HEAP_SIZE: usize = 128 * 1024;
#[repr(align(4096))]
struct Heap([u8; HEAP_SIZE]);
static mut HEAP: Heap = Heap([0; HEAP_SIZE]);
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Bump;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = addr_of_mut!(HEAP) as *mut u8 as usize;
        let mut current = NEXT.load(Ordering::Relaxed);
        loop {
            let Some(start) = base.checked_add(current).and_then(|n| n.checked_add(layout.align() - 1))
                .map(|n| (n & !(layout.align() - 1)) - base) else { return core::ptr::null_mut() };
            let Some(end) = start.checked_add(layout.size()) else { return core::ptr::null_mut() };
            if end > HEAP_SIZE { return core::ptr::null_mut() }
            match NEXT.compare_exchange_weak(current, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return (addr_of_mut!(HEAP) as *mut u8).add(start),
                Err(next) => current = next,
            }
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator]
static ALLOC: Bump = Bump;

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(ok: bool, message: &str) { if !ok { write("FAIL "); write(message); write("\r\n"); halt() } }

// This explicitly simulated transport refuses FEATURES_OK. It is not a QEMU device.
struct RejectingTransport { status: DeviceStatus, page_size_calls: usize, queue_calls: usize, driver_ok_calls: usize }
impl RejectingTransport { fn new() -> Self { Self { status: DeviceStatus::empty(), page_size_calls: 0, queue_calls: 0, driver_ok_calls: 0 } } }
impl Transport for RejectingTransport {
    fn device_type(&self) -> DeviceType { DeviceType::EntropySource }
    fn read_device_features(&mut self) -> u64 { Feature::VERSION_1.bits() }
    fn write_driver_features(&mut self, _features: u64) {}
    fn max_queue_size(&mut self, _queue: u16) -> u32 { 8 }
    fn notify(&mut self, _queue: u16) {}
    fn get_status(&self) -> DeviceStatus { self.status }
    fn set_status(&mut self, status: DeviceStatus) {
        if status.contains(DeviceStatus::DRIVER_OK) { self.driver_ok_calls += 1; }
        self.status = status & !DeviceStatus::FEATURES_OK;
    }
    fn set_guest_page_size(&mut self, _size: u32) { self.page_size_calls += 1; }
    fn requires_legacy_layout(&self) -> bool { false }
    fn queue_set(&mut self, _q: u16, _s: u32, _d: PhysAddr, _a: PhysAddr, _u: PhysAddr) { self.queue_calls += 1; }
    fn queue_unset(&mut self, _q: u16) {}
    fn queue_used(&mut self, _q: u16) -> bool { false }
    fn ack_interrupt(&mut self) -> InterruptStatus { InterruptStatus::empty() }
    fn read_config_generation(&self) -> u32 { 0 }
    fn read_config_space<T: FromBytes + IntoBytes>(&self, _offset: usize) -> virtio_drivers::Result<T> { Err(virtio_drivers::Error::Unsupported) }
    fn write_config_space<T: IntoBytes + Immutable>(&mut self, _offset: usize, _value: T) -> virtio_drivers::Result<()> { Err(virtio_drivers::Error::Unsupported) }
}

#[derive(Debug, PartialEq, Eq)]
enum NegotiationError { LegacyUnsupported, ResetIncomplete, VersionMissing, FeaturesRejected }

// Experimental modern-only sequence, deliberately separate from upstream.
// Returns after FEATURES_OK is re-read; it does not initialize queues or DRIVER_OK.
fn checked_begin<T: Transport>(device: &mut T) -> Result<Feature, NegotiationError> {
    if device.requires_legacy_layout() { return Err(NegotiationError::LegacyUnsupported); }
    device.set_status(DeviceStatus::empty());
    if !device.get_status().is_empty() { return Err(NegotiationError::ResetIncomplete); }
    device.set_status(DeviceStatus::ACKNOWLEDGE);
    device.set_status(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER);
    let offered = device.read_device_features();
    if offered & Feature::VERSION_1.bits() == 0 {
        device.set_status(device.get_status() | DeviceStatus::FAILED);
        return Err(NegotiationError::VersionMissing);
    }
    let chosen = Feature::VERSION_1;
    device.write_driver_features(chosen.bits());
    device.set_status(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER | DeviceStatus::FEATURES_OK);
    let status = device.get_status();
    if !status.contains(DeviceStatus::FEATURES_OK) || status.contains(DeviceStatus::DEVICE_NEEDS_RESET) {
        device.set_status(status | DeviceStatus::FAILED);
        return Err(NegotiationError::FeaturesRejected);
    }
    Ok(chosen)
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut baseline = RejectingTransport::new();
    let features = baseline.begin_init(Feature::VERSION_1);
    check(features.contains(Feature::VERSION_1), "upstream return");
    check(!baseline.get_status().contains(DeviceStatus::FEATURES_OK), "mock did not reject");
    check(baseline.page_size_calls == 1, "upstream continuation not observed");
    write("MOCK upstream_begin_init=returned_features device_features_ok=absent page_size_called=yes\r\n");
    let mut fixed = RejectingTransport::new();
    check(checked_begin(&mut fixed) == Err(NegotiationError::FeaturesRejected), "checked rejection");
    check(fixed.get_status().contains(DeviceStatus::FAILED), "failed status");
    check(fixed.page_size_calls == 0 && fixed.queue_calls == 0 && fixed.driver_ok_calls == 0, "continued after rejection");
    write("MOCK checked_begin=rejected failed=yes queue_setup=no driver_ok=no\r\n");
    for i in 0..32 {
        let ptr = NonNull::new((0x0a00_0000usize + i * 0x200) as *mut VirtIOHeader).unwrap();
        let Ok(mut device) = (unsafe { MmioTransport::new(ptr, 0x200) }) else { continue };
        if device.device_type() != DeviceType::EntropySource { continue; }
        if device.requires_legacy_layout() {
            check(checked_begin(&mut device) == Err(NegotiationError::LegacyUnsupported), "legacy did not fail closed");
            write("QEMU legacy checked_begin=unsupported\r\nFAIL QEMU device legacy\r\n");
            halt();
        }
        check(checked_begin(&mut device) == Ok(Feature::VERSION_1), "QEMU acceptance");
        check(device.get_status().contains(DeviceStatus::FEATURES_OK), "QEMU status did not retain features");
        check(!device.get_status().contains(DeviceStatus::DRIVER_OK), "unexpected driver ready");
        write("QEMU modern_rng features_ok=accepted driver_ok=no queue_setup=no\r\n");
        device.set_status(DeviceStatus::empty());
        write("PASS virtio feature negotiation scenario\r\n");
        halt();
    }
    write("FAIL no modern QEMU virtio rng\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { write("FAIL kernel panic\r\n"); halt() }
