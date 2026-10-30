#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use local_inference_sketch::{infer, Input, Output, OutOfRange};

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
    let cases=[
        (Input::new(0,0,0).unwrap(), Output {hidden:[0,20,0,30],scores:[-40,30]}),
        (Input::new(100,100,100).unwrap(), Output {hidden:[150,220,400,30],scores:[460,880]}),
        (Input::new(70,20,10).unwrap(), Output {hidden:[130,20,80,0],scores:[330,30]}),
        (Input::new(100,0,0).unwrap(), Output {hidden:[250,0,100,0],scores:[610,-70]}),
    ];
    for (input,expected) in cases {check(infer(input)==expected && infer(input)==expected,"exact or repeat inference");}
    write("CASE input=0,0,0 hidden=0,20,0,30 scores=-40,30\r\n");
    write("CASE input=100,100,100 hidden=150,220,400,30 scores=460,880\r\n");
    write("CASE input=70,20,10 hidden=130,20,80,0 scores=330,30\r\n");
    write("CASE input=100,0,0 hidden=250,0,100,0 scores=610,-70\r\n");
    check(Input::new(-1,0,0)==Err(OutOfRange),"negative pressure");
    check(Input::new(101,0,0)==Err(OutOfRange),"high pressure");
    check(Input::new(0,-1,0)==Err(OutOfRange),"negative CPU");
    check(Input::new(0,101,0)==Err(OutOfRange),"high CPU");
    check(Input::new(0,0,-1)==Err(OutOfRange),"negative IO");
    check(Input::new(0,0,101)==Err(OutOfRange),"high IO");
    for p in [0,100] {for c in [0,100] {for io in [0,100] {
        let out=infer(Input::new(p,c,io).unwrap());
        check(out.hidden.iter().zip([350,420,400,330]).all(|(&v,max)| v>=0 && v<=max),"hidden bound");
        check((-740..=1110).contains(&out.scores[0]) && (-370..=1530).contains(&out.scores[1]),"score bound");
    }}}
    write("CHECK repeat=deterministic invalid_inputs=rejected corner_bounds=ok control=none\r\n");
    write("PASS synthetic integer neural inference\r\n");
    halt();
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! { write("FAIL kernel panic\r\n"); halt() }
