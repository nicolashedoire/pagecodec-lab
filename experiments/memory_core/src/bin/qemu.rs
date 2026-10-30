#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use memory_core_sketch::{FrameAllocator, ReservedRange, FRAME_SIZE, MAX_FRAMES, InitError, FreeError};

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

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(condition: bool, name: &str) {
    if !condition { write("FAIL "); write(name); write("\r\n"); halt(); }
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    // These are synthetic addresses. The test never dereferences them.
    let base = 0x5000_0000usize;
    let frame = FRAME_SIZE;
    let reserved = [ReservedRange { start: base + 2 * frame, end: base + 4 * frame }];
    let mut pool = FrameAllocator::new(base, 6, &reserved).unwrap();
    check(pool.allocate() == Some(base), "first allocation");
    check(pool.allocate() == Some(base + frame), "second allocation");
    check(pool.allocate() == Some(base + 4 * frame), "skip reserved frames");
    check(pool.allocate() == Some(base + 5 * frame), "last frame");
    check(pool.allocate().is_none(), "exhausted pool");
    check(pool.free(base + 2 * frame) == Err(FreeError::Reserved), "free reserved");
    check(pool.free(base + 1) == Err(FreeError::Unaligned), "free unaligned");
    check(pool.free(base - frame) == Err(FreeError::OutsidePool), "below pool");
    check(pool.free(base + 6 * frame) == Err(FreeError::OutsidePool), "at pool end");
    check(pool.free(base + frame) == Ok(()), "valid free");
    check(pool.free(base + frame) == Err(FreeError::NotAllocated), "double free");
    check(pool.allocate() == Some(base + frame) && pool.allocate().is_none(), "reallocation");
    write("CHECK allocations=4 reserved_skipped=2 exhaustion=ok reallocation=ok\r\n");
    write("CHECK invalid_free=reserved,unaligned,outside,double-free\r\n");

    check(matches!(FrameAllocator::new(base + 1, 1, &[]), Err(InitError::UnalignedBase)), "base alignment");
    check(matches!(FrameAllocator::new(base, 0, &[]), Err(InitError::InvalidFrameCount)), "zero count");
    check(matches!(FrameAllocator::new(base, MAX_FRAMES + 1, &[]), Err(InitError::InvalidFrameCount)), "too many frames");
    check(matches!(FrameAllocator::new(usize::MAX - FRAME_SIZE + 1, 1, &[]), Err(InitError::AddressOverflow)), "address overflow");
    let bad = [ReservedRange { start: base + 1, end: base + frame }];
    check(matches!(FrameAllocator::new(base, 6, &bad), Err(InitError::InvalidReservation)), "unaligned reserve");
    let bad = [ReservedRange { start: base, end: base }];
    check(matches!(FrameAllocator::new(base, 6, &bad), Err(InitError::InvalidReservation)), "empty reserve");
    let bad = [ReservedRange { start: base, end: base + 7 * frame }];
    check(matches!(FrameAllocator::new(base, 6, &bad), Err(InitError::InvalidReservation)), "reserve outside pool");
    write("CHECK invalid_pools_and_reservations=rejected\r\n");

    // Exercise bitmap-word and fixed-capacity boundaries with overlapping reservations.
    let reserved = [ReservedRange { start: base + 63 * frame, end: base + 65 * frame },
                    ReservedRange { start: base + 64 * frame, end: base + 66 * frame }];
    let mut big = FrameAllocator::new(base, MAX_FRAMES, &reserved).unwrap();
    for i in 0..MAX_FRAMES {
        if (63..66).contains(&i) { continue; }
        check(big.allocate() == Some(base + i * frame), "bitmap boundary sequence");
    }
    check(big.allocate().is_none(), "large pool exhaustion");
    check(big.free(base + 64 * frame) == Err(FreeError::Reserved), "overlap reserved");
    check(big.free(base + (MAX_FRAMES - 1) * frame) == Ok(()), "free last frame");
    check(big.allocate() == Some(base + (MAX_FRAMES - 1) * frame), "reallocate last frame");
    write("CHECK bitmap_boundaries=63,64,2047 overlapping_reservations=ok\r\n");
    write("PASS physical frame state synthetic scenario\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
