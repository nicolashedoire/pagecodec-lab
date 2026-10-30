#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::addr_of_mut;
use memory_core_sketch::{FrameAllocator, ReservedRange, FreeError, FRAME_SIZE};

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

const FRAME_COUNT: usize = 6;
const ARENA_BYTES: usize = FRAME_COUNT * FRAME_SIZE;
#[repr(C, align(4096))]
struct Arena([u8; ARENA_BYTES]);
// The sole access path below is via raw pointers; no Rust references to this
// mutable static are created. The demo runs with interrupts masked on one CPU.
static mut ARENA: Arena = Arena([0; ARENA_BYTES]);

fn write(s: &str) {
    for b in s.bytes() { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
fn decimal(mut value: usize) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 { break; }
    }
    for &b in &digits[i..] { unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }}
}
// Synthetic metadata only: relative frame index and pattern seed, never addresses or bytes.
fn trace(action: &str, index: Option<usize>, seed: Option<u8>, result: &str) {
    write("IO action="); write(action); write(" frame=");
    if let Some(i) = index { decimal(i); } else { write("none"); }
    write(" seed=");
    if let Some(v) = seed { decimal(v as usize); } else { write("none"); }
    write(" result="); write(result); write("\r\n");
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }
fn check(condition: bool, name: &str) {
    if !condition { write("FAIL "); write(name); write("\r\n"); halt() }
}

fn offset_of(address: usize, base: usize) -> Option<usize> {
    let offset = address.checked_sub(base)?;
    if offset % FRAME_SIZE != 0 || offset.checked_add(FRAME_SIZE)? > ARENA_BYTES { return None; }
    Some(offset)
}

fn pattern(offset: usize, seed: u8) -> u8 {
    (offset as u8).wrapping_mul(37).wrapping_add(seed)
}

// Use pointer arithmetic from ARENA's original pointer, not by converting the
// integer address back into a pointer. The checked address is a whole frame
// inside that allocation. Volatile makes the actual guest accesses observable.
fn fill(arena: *mut u8, base: usize, address: usize, seed: u8) -> bool {
    let Some(offset) = offset_of(address, base) else { return false };
    for i in 0..FRAME_SIZE { unsafe { arena.add(offset + i).write_volatile(pattern(i, seed)); } }
    true
}
fn matches(arena: *mut u8, base: usize, address: usize, seed: u8) -> bool {
    let Some(offset) = offset_of(address, base) else { return false };
    for i in 0..FRAME_SIZE {
        if unsafe { arena.add(offset + i).read_volatile() } != pattern(i, seed) { return false; }
    }
    true
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let arena = addr_of_mut!(ARENA).cast::<u8>();
    let base = arena as usize;
    check(base % FRAME_SIZE == 0, "arena alignment");
    let Some(end) = base.checked_add(ARENA_BYTES) else { check(false, "arena end overflow"); halt() };
    // The reserved third page is part of this guest-owned static arena only.
    let reserved = base + 2 * FRAME_SIZE;
    let initialized = fill(arena, base, reserved, 0xa5);
    trace("initialize-reserved", Some(2), Some(0xa5), if initialized { "ok" } else { "error" });
    check(initialized, "reserved initialization");
    let ranges = [ReservedRange { start: reserved, end: reserved + FRAME_SIZE }];
    let mut allocator = FrameAllocator::new(base, FRAME_COUNT, &ranges).unwrap();
    let mut frames = [0usize; FRAME_COUNT - 1];
    for (i, frame) in frames.iter_mut().enumerate() {
        let allocated = allocator.allocate();
        let Some(address) = allocated else { trace("allocate", None, None, "unexpected-exhaustion"); check(false, "early exhaustion"); halt() };
        let valid = address >= base && address < end && address != reserved && offset_of(address, base).is_some();
        let index = offset_of(address, base).map(|offset| offset / FRAME_SIZE);
        trace("allocate", index, None, if valid { "ok" } else { "error" });
        check(valid, "reserved or outside allocation");
        *frame = address;
        let seed = (i as u8) + 1;
        let filled = fill(arena, base, address, seed);
        trace("write", index, Some(seed), if filled { "ok" } else { "error" });
        check(filled, "frame fill bounds");
    }
    let exhausted = allocator.allocate().is_none();
    trace("allocate", None, None, if exhausted { "exhausted" } else { "error" });
    check(exhausted, "missing exhaustion");
    for (i, address) in frames.iter().copied().enumerate() {
        let seed = (i as u8) + 1;
        let verified = matches(arena, base, address, seed);
        trace("read-verify", Some((address - base) / FRAME_SIZE), Some(seed), if verified { "ok" } else { "error" });
        check(verified, "initial pattern or alias");
    }
    let reserved_intact = matches(arena, base, reserved, 0xa5);
    trace("read-reserved", Some(2), Some(0xa5), if reserved_intact { "ok" } else { "error" });
    check(reserved_intact, "reserved pattern changed");
    let out_of_range = !fill(arena, base, end, 9);
    trace("write", Some(FRAME_COUNT), Some(9), if out_of_range { "rejected-outside" } else { "error" });
    let unaligned = !fill(arena, base, base + 1, 9);
    trace("write", None, Some(9), if unaligned { "rejected-unaligned" } else { "error" });
    check(out_of_range && unaligned, "bounds validation");
    let rejected_reserved = allocator.free(reserved) == Err(FreeError::Reserved);
    trace("free", Some(2), None, if rejected_reserved { "rejected-reserved" } else { "error" });
    check(rejected_reserved, "reserved free");
    let rejected_outside = allocator.free(end) == Err(FreeError::OutsidePool);
    trace("free", Some(FRAME_COUNT), None, if rejected_outside { "rejected-outside" } else { "error" });
    check(rejected_outside, "outside free");
    write("CHECK arena_aligned=yes allocated_frames=5 reserved_skipped=yes exhaustion=ok\r\n");
    write("CHECK distinct_patterns=5 reserved_pattern=intact bounds=rejected\r\n");

    let recycled = frames[1];
    let recycled_index = (recycled - base) / FRAME_SIZE;
    let freed = allocator.free(recycled).is_ok();
    trace("free", Some(recycled_index), None, if freed { "ok" } else { "error" });
    check(freed, "free");
    let double_rejected = allocator.free(recycled) == Err(FreeError::NotAllocated);
    trace("free", Some(recycled_index), None, if double_rejected { "rejected-double-free" } else { "error" });
    check(double_rejected, "double free");
    let reallocated = allocator.allocate() == Some(recycled);
    trace("reallocate", Some(recycled_index), None, if reallocated { "ok" } else { "error" });
    check(reallocated, "reallocate");
    let filled = fill(arena, base, recycled, 0xe7);
    trace("write", Some(recycled_index), Some(0xe7), if filled { "ok" } else { "error" });
    check(filled, "recycled fill");
    for (i, address) in frames.iter().copied().enumerate() {
        let seed = if i == 1 { 0xe7 } else { (i as u8) + 1 };
        let verified = matches(arena, base, address, seed);
        trace("read-verify", Some((address - base) / FRAME_SIZE), Some(seed), if verified { "ok" } else { "error" });
        check(verified, "pattern after recycle");
    }
    let reserved_intact = matches(arena, base, reserved, 0xa5);
    trace("read-reserved", Some(2), Some(0xa5), if reserved_intact { "ok" } else { "error" });
    check(reserved_intact, "reserved pattern after recycle");
    write("CHECK free_reallocate=ok other_patterns=intact reserved_pattern=intact\r\n");
    write("PASS guest static arena frame scenario\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
