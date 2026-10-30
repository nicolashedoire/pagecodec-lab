//! Controlled QEMU EL0 timer probe, run once before the graphical guest.
use alloc::vec::Vec;
use serde_json::{json,Value};
use core::{arch::{asm,global_asm},ptr::{addr_of,addr_of_mut,read_volatile,write_volatile}};
const PAGE:usize=4096;
const GICD:usize=0x0800_0000;const GICC:usize=0x0801_0000;
#[repr(C,align(4096))] struct UserStack([u8;PAGE]);
#[repr(C,align(16))] #[derive(Clone,Copy)] struct Frame {regs:[u64;31],sp:u64,elr:u64,spsr:u64}
#[repr(C)] #[derive(Clone,Copy)] struct Event {actor:u64,count:u64,pc:u64,spsr:u64,sp:u64,next:u64,kernel_sp:u64}
const EMPTY:Frame=Frame{regs:[0;31],sp:0,elr:0,spsr:0};
#[link_section=".userstack"] static mut A_STACK:UserStack=UserStack([0;PAGE]);
#[link_section=".userstack"] static mut B_STACK:UserStack=UserStack([0;PAGE]);
static mut KERNEL_DATA:u64=0x0123_4567_89ab_cdef;
static mut A_FRAME:Frame=EMPTY;static mut B_FRAME:Frame=EMPTY;
#[no_mangle] static mut SAVED:[u64;2]=[0;2];
#[no_mangle] static mut CURRENT:u64=0;
#[no_mangle] static mut TASK_STATE:[u64;2]=[0;2];
#[no_mangle] static mut READY_QUEUE:[u64;2]=[0;2];
#[no_mangle] static mut READY_HEAD:u64=0;
#[no_mangle] static mut READY_LEN:u64=0;
#[no_mangle] static mut TICKS:u64=0;
#[no_mangle] static mut DELTA:u64=0;
#[no_mangle] static mut EVENTS:[Event;8]=[Event{actor:0,count:0,pc:0,spsr:0,sp:0,next:2,kernel_sp:0};8];
static mut PROBE_DONE:u64=0;
extern "C" {
 static __image_start:u8;static __image_end:u8;static __text_end:u8;
 static __usertext_start:u8;static __usertext_end:u8;static __userstack_start:u8;static __userstack_end:u8;
 static __bss_end:u8;static __stack_top:u8;static vector_table:u8;
 static task_a_loop:u8;static task_a_end:u8;static task_b_loop:u8;static task_b_end:u8;
 fn start_first(frame:*const Frame)->!;fn resume_graphics()->!;
}
fn halt()->! {loop{unsafe{asm!("wfe",options(nomem,nostack))}}}
fn check(ok:bool,msg:&str){if !ok{super::write("FAIL EL0 ");super::write(msg);super::write("\r\n");halt()}}
fn physical(value:usize)->usize {let out:usize;unsafe{asm!("mov {out}, {value}",out=out(reg) out,value=in(reg) value,options(nomem,nostack,preserves_flags));}out}
fn frame(actor:u64,sp:usize,pc:usize)->Frame {
 let mut f=EMPTY;for i in 0..31 {f.regs[i]=1000+actor*100+i as u64;}f.regs[19]=0;f.sp=sp as u64;f.elr=pc as u64;
 f.spsr=if actor==0 {0xa000_0340}else{0x5000_0340};f
}

pub fn start()->! {
 let start=physical(addr_of!(__image_start) as usize);let end=physical(addr_of!(__image_end) as usize);
 let text_end=physical(addr_of!(__text_end) as usize);let ut0=physical(addr_of!(__usertext_start) as usize);let ut1=physical(addr_of!(__usertext_end) as usize);
 let us0=physical(addr_of!(__userstack_start) as usize);let us1=physical(addr_of!(__userstack_end) as usize);
 let a=physical(addr_of_mut!(A_STACK) as usize);let b=physical(addr_of_mut!(B_STACK) as usize);
 let kb=physical(addr_of!(__bss_end) as usize);let kt=physical(addr_of!(__stack_top) as usize);
 let vector=physical(addr_of!(vector_table) as usize);
 let apc=physical(addr_of!(task_a_loop) as usize);let aend=physical(addr_of!(task_a_end) as usize);
 let bpc=physical(addr_of!(task_b_loop) as usize);let bend=physical(addr_of!(task_b_end) as usize);
 let mut sp:usize;let mut sctlr:u64;unsafe{asm!("mov {},sp",out(reg) sp);asm!("mrs {},SCTLR_EL1",out(reg) sctlr);}
 check(sctlr&1!=0 && sctlr&((1<<2)|(1<<12))==0 && sp>kb && sp<=kt && sp%16==0,"MMU or kernel stack");
 check(start<text_end && text_end<=ut0 && ut0<ut1 && us0<us1 && us1==end && kt<=us0,"section bounds");
 check(ut0%PAGE==0 && ut0.checked_add(PAGE)==Some(ut1) && us0.checked_add(2*PAGE)==Some(us1) && a%PAGE==0 && b%PAGE==0 && a>=us0 && b>=us0 && a+PAGE<=us1 && b+PAGE<=us1 && (a+PAGE<=b || b+PAGE<=a),"EL0 stack bounds");
 check(apc>=ut0 && apc<aend && aend<=ut1 && bpc>=ut0 && bpc<bend && bend<=ut1,"EL0 code bounds");
 check(vector%2048==0 && vector>=start && vector+2048<=text_end,"VBAR bounds");
 check(core::mem::size_of::<Frame>()==272 && core::mem::size_of::<Event>()==56,"frame layout");
 for p in [addr_of_mut!(A_STACK).cast::<u8>(),addr_of_mut!(B_STACK).cast::<u8>()] {for i in 0..PAGE {unsafe{write_volatile(p.add(i),0)}}}
 unsafe{asm!("msr VBAR_EL1, {}","isb",in(reg) vector);}
 let mut vbar:u64;let mut freq:u64;unsafe{asm!("mrs {},VBAR_EL1",out(reg) vbar);asm!("mrs {},CNTFRQ_EL0",out(reg) freq);}
 check(vbar==vector as u64,"VBAR readback");let delta=freq/100;check(delta>0 && delta<=i32::MAX as u64,"timer frequency");
 unsafe {
  addr_of_mut!(A_FRAME).write(frame(0,a+PAGE,addr_of!(task_a_loop) as usize));addr_of_mut!(B_FRAME).write(frame(1,b+PAGE,addr_of!(task_b_loop) as usize));
  addr_of_mut!(SAVED).cast::<u64>().write(addr_of!(A_FRAME) as u64);addr_of_mut!(SAVED).cast::<u64>().add(1).write(addr_of!(B_FRAME) as u64);
  addr_of_mut!(TASK_STATE).cast::<u64>().write(2);addr_of_mut!(TASK_STATE).cast::<u64>().add(1).write(1);
  addr_of_mut!(READY_QUEUE).cast::<u64>().write(1);addr_of_mut!(READY_HEAD).write(0);addr_of_mut!(READY_LEN).write(1);addr_of_mut!(DELTA).write(delta);
  write_volatile((GICD+0x000) as *mut u32,0);write_volatile((GICC+0x000) as *mut u32,0);
  write_volatile((GICD+0x180) as *mut u32,1<<27);write_volatile((GICD+0x280) as *mut u32,1<<27);
  write_volatile((GICD+0x400+27) as *mut u8,0x80);write_volatile((GICC+0x004) as *mut u32,0xff);
  write_volatile((GICD+0x100) as *mut u32,1<<27);write_volatile((GICD+0x000) as *mut u32,1);write_volatile((GICC+0x000) as *mut u32,1);
  asm!("dsb sy","isb");asm!("msr CNTV_TVAL_EL0, {}",in(reg) delta);asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);asm!("isb");
  start_first(addr_of!(A_FRAME));
 }
}

pub fn status()->Value {
 let done=unsafe{read_volatile(addr_of!(PROBE_DONE))}==1;
 let events:Vec<Value>=(0..8).map(|i|{let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};json!({"task":e.actor,"count":e.count,"next":e.next})}).collect();
 json!({"validated":done,"ticks":8,"el":"EL0","events":events,"timer":"disabled","interrupts":"masked"})
}

global_asm!(r#"
    .section .text.vectors, "ax"
    .balign 2048
    .global vector_table
vector_table:
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal             // lower EL AArch64 sync
    .balign 128
    b lower_el_irq          // lower EL AArch64 IRQ
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128

    .section .text.irq, "ax"
lower_el_irq:
    // Exception arrived from EL0 on SP_EL1. Save every integer register,
    // user SP and exception return state before using any as scratch.
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, SP_EL0
    str x0, [sp, #248]
    mrs x0, ELR_EL1
    str x0, [sp, #256]
    mrs x0, SPSR_EL1
    str x0, [sp, #264]
    and x1, x0, #0xf
    cbnz x1, irq_fatal
    mov x1, #0x08010000
    ldr w0, [x1, #0x0c]
    and w2, w0, #0x3ff
    cmp w2, #27
    b.ne irq_fatal
    msr CNTV_CTL_EL0, xzr
    isb
    dsb sy
    str w0, [x1, #0x10]
    dsb sy
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    ldr x2, [x0]
    cmp x2, #1
    b.hi irq_fatal
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    ldr x3, [x3, x2, lsl #3]
    // Copy the kernel-stack exception frame to this actor's preallocated frame.
    mov x4, sp
    mov x5, #34
1:  ldr x6, [x4], #8
    str x6, [x3], #8
    subs x5, x5, #1
    b.ne 1b
    adrp x5, TICKS
    add x5, x5, :lo12:TICKS
    ldr x6, [x5]
    cmp x6, #8
    b.hs irq_fatal
    adrp x7, EVENTS
    add x7, x7, :lo12:EVENTS
    mov x8, #56
    madd x7, x6, x8, x7
    str x2, [x7, #0]
    ldr x8, [sp, #152]
    str x8, [x7, #8]
    ldr x8, [sp, #256]
    str x8, [x7, #16]
    ldr x8, [sp, #264]
    str x8, [x7, #24]
    ldr x8, [sp, #248]
    str x8, [x7, #32]
    mov x8, #2
    str x8, [x7, #40]
    add x8, sp, #272
    str x8, [x7, #48]
    add x6, x6, #1
    str x6, [x5]
    cmp x6, #8
    b.eq all_done
    // RUNNING -> READY, enqueue current ID, then dequeue the oldest ready ID.
    adrp x4, TASK_STATE
    add x4, x4, :lo12:TASK_STATE
    ldr x5, [x4, x2, lsl #3]
    cmp x5, #2
    b.ne irq_fatal
    mov x5, #1
    str x5, [x4, x2, lsl #3]
    adrp x4, READY_LEN
    add x4, x4, :lo12:READY_LEN
    ldr x5, [x4]
    cmp x5, #1
    b.hi irq_fatal
    adrp x6, READY_HEAD
    add x6, x6, :lo12:READY_HEAD
    ldr x6, [x6]
    cmp x6, #1
    b.hi irq_fatal
    add x6, x6, x5
    and x6, x6, #1
    adrp x7, READY_QUEUE
    add x7, x7, :lo12:READY_QUEUE
    str x2, [x7, x6, lsl #3]
    add x5, x5, #1
    str x5, [x4]
    bl select_ready
    cmp x0, #2
    b.hs irq_fatal
    adrp x9, TICKS
    add x9, x9, :lo12:TICKS
    ldr x9, [x9]
    sub x9, x9, #1
    adrp x10, EVENTS
    add x10, x10, :lo12:EVENTS
    mov x11, #56
    madd x10, x9, x11, x10
    str x0, [x10, #40]
    mov x2, x0
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    str x2, [x0]
    adrp x5, DELTA
    add x5, x5, :lo12:DELTA
    ldr x5, [x5]
    msr CNTV_TVAL_EL0, x5
    mov x5, #1
    msr CNTV_CTL_EL0, x5
    isb
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    ldr x0, [x3, x2, lsl #3]
    add sp, sp, #272       // release kernel IRQ frame before returning to EL0
    b restore_user_frame
all_done:
    // Timer is disabled; keep every interrupt class masked for validation.
    msr daifset, #0xf
    b el0_finish

select_ready:
    adrp x1, READY_LEN
    add x1, x1, :lo12:READY_LEN
    ldr x2, [x1]
    cbz x2, select_empty
    cmp x2, #2
    b.hi irq_fatal
    adrp x3, READY_HEAD
    add x3, x3, :lo12:READY_HEAD
    ldr x4, [x3]
    cmp x4, #1
    b.hi irq_fatal
    adrp x5, READY_QUEUE
    add x5, x5, :lo12:READY_QUEUE
    ldr x0, [x5, x4, lsl #3]
    cmp x0, #2
    b.hs irq_fatal
    adrp x5, TASK_STATE
    add x5, x5, :lo12:TASK_STATE
    ldr x6, [x5, x0, lsl #3]
    cmp x6, #1
    b.ne irq_fatal
    mov x6, #2
    str x6, [x5, x0, lsl #3]
    eor x4, x4, #1
    str x4, [x3]
    sub x2, x2, #1
    str x2, [x1]
    ret
select_empty:
    mov x0, #2
    ret
irq_fatal:
    msr daifset, #0xf
2:  wfe
    b 2b

    .global start_first
start_first:
restore_user_frame:
    mov x30, x0
    ldr x1, [x30, #248]
    msr SP_EL0, x1
    ldr x1, [x30, #256]
    msr ELR_EL1, x1
    ldr x1, [x30, #264]
    msr SPSR_EL1, x1
    ldp x0, x1, [x30, #0]
    ldp x2, x3, [x30, #16]
    ldp x4, x5, [x30, #32]
    ldp x6, x7, [x30, #48]
    ldp x8, x9, [x30, #64]
    ldp x10, x11, [x30, #80]
    ldp x12, x13, [x30, #96]
    ldp x14, x15, [x30, #112]
    ldp x16, x17, [x30, #128]
    ldp x18, x19, [x30, #144]
    ldp x20, x21, [x30, #160]
    ldp x22, x23, [x30, #176]
    ldp x24, x25, [x30, #192]
    ldp x26, x27, [x30, #208]
    ldr x28, [x30, #224]
    ldr x29, [x30, #232]
    ldr x30, [x30, #240]
    eret

    .section .text.resume, "ax"
    .global resume_graphics
resume_graphics:
    ldr x0, =__stack_top
    mov sp, x0
    b graph_main

    .section .usertext, "ax"
    .global task_a_loop
    .global task_a_end
task_a_loop:
    add x19, x19, #1
    b task_a_loop
task_a_end:
    .global task_b_loop
    .global task_b_end
task_b_loop:
    add x19, x19, #1
    b task_b_loop
task_b_end:
"#);

#[no_mangle]
pub extern "C" fn el0_finish()->! {
    check(unsafe{read_volatile(addr_of!(TICKS))}==8,"tick count");
    let kb=physical(addr_of!(__bss_end) as usize);let kt=physical(addr_of!(__stack_top) as usize);
    let stacks=[physical(addr_of!(A_STACK) as usize),physical(addr_of!(B_STACK) as usize)];
    let mut previous=[0u64;2];
    for i in 0..8 {
        let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};let actor=(i%2) as u64;
        let (lo,hi)=if actor==0 {(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64)} else {(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64)};
        let expected_spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(e.actor==actor && e.next==if i<7 {1-actor} else {2},"selected actor or next");
        check(e.count>previous[actor as usize],"task did not progress");previous[actor as usize]=e.count;
        check(e.pc>=lo && e.pc<hi && e.pc%4==0,"user PC");
        check(e.spsr&0xf000_03ff==expected_spsr && e.sp as usize==stacks[actor as usize]+PAGE,"user SPSR or SP");
        check(e.kernel_sp%16==0 && e.kernel_sp as usize>kb && (e.kernel_sp as usize)<=kt && (e.kernel_sp as usize).checked_sub(272).is_some_and(|n|n>=kb),"kernel IRQ stack");

    }
    for actor in 0..2usize {
        let saved=unsafe{read_volatile(addr_of!(SAVED).cast::<u64>().add(actor))} as *const Frame;
        let expected=if actor==0 {addr_of!(A_FRAME)} else {addr_of!(B_FRAME)};
        check(saved==expected,"frame pointer");let f=unsafe{read_volatile(expected)};
        let spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(f.sp as usize==stacks[actor]+PAGE && f.spsr&0xf000_03ff==spsr,"final user frame");
        for i in 0..31 {if i!=19 {check(f.regs[i]==1000+actor as u64*100+i as u64,"GPR corruption")}}
        check(f.regs[19]==previous[actor],"counter mismatch");
    }
    let current=unsafe{read_volatile(addr_of!(CURRENT))} as usize;
    let head=unsafe{read_volatile(addr_of!(READY_HEAD))} as usize;
    let len=unsafe{read_volatile(addr_of!(READY_LEN))};
    check(current==1 && head<2 && len==1,"terminal ring shape");
    check(unsafe{read_volatile(addr_of!(READY_QUEUE).cast::<u64>().add(head))}==0
          && unsafe{read_volatile(addr_of!(TASK_STATE).cast::<u64>())}==1
          && unsafe{read_volatile(addr_of!(TASK_STATE).cast::<u64>().add(1))}==2,"terminal states");
    check(unsafe{read_volatile(addr_of!(KERNEL_DATA))}==0x0123_4567_89ab_cdef,"kernel data changed");
    let mut ctl:u64;unsafe{asm!("mrs {}, CNTV_CTL_EL0",out(reg) ctl);}
    let active=unsafe{read_volatile((GICD+0x300) as *const u32)};
    check(ctl&1==0 && active&(1<<27)==0,"timer or GIC active");
    unsafe {
        write_volatile((GICD+0x180) as *mut u32,1<<27);
        write_volatile((GICC+0x000) as *mut u32,0);
        write_volatile((GICD+0x000) as *mut u32,0);
        asm!("dsb sy","isb");
        write_volatile(addr_of_mut!(PROBE_DONE),1);
        resume_graphics();
    }
}
