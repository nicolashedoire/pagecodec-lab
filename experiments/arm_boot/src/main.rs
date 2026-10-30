#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

// QEMU's virt machine enters at EL1 with virtualization disabled. Initialize our
// own stack and zero BSS before calling Rust. This is not Apple Silicon boot.
global_asm!(
    r#"
    .section .text.boot, "ax"
    .global _start
_start:
    msr daifset, #0xf
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
"#
);

const UART_DR: *mut u32 = 0x0900_0000 as *mut u32;
const UART_FR: *const u32 = 0x0900_0018 as *const u32;
const TX_FULL: u32 = 1 << 5;

fn write_byte(byte: u8) {
    // SAFETY: QEMU virt exposes a PL011 at this address. These are device
    // registers, so reads and writes must be volatile. The image must only be
    // run on this declared virtual platform.
    unsafe {
        while UART_FR.read_volatile() & TX_FULL != 0 {
            core::hint::spin_loop();
        }
        UART_DR.write_volatile(byte as u32);
    }
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    for &byte in b"ARM Rust kernel: boot OK\r\n" {
        write_byte(byte);
    }
    loop {
        // SAFETY: WFE is an architectural low-power wait instruction. We do
        // not install interrupts or rely on it waking up in this first image.
        unsafe { asm!("wfe", options(nomem, nostack)) }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    for &byte in b"ARM Rust kernel: panic\r\n" {
        write_byte(byte);
    }
    loop {
        // SAFETY: see rust_main above.
        unsafe { asm!("wfe", options(nomem, nostack)) }
    }
}
