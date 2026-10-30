#![no_std]
#![no_main]

use core::{arch::{asm, global_asm}, panic::PanicInfo, ptr::{addr_of, addr_of_mut, read_volatile, write_volatile}};

global_asm!(r#"
    .section .text.boot, "ax"
    .global _start
_start:
    msr daifset, #0xf
    mrs x0, CurrentEL
    cmp x0, #4
    b.ne fatal_halt
    mrs x0, SCTLR_EL1
    tbnz x0, #0, fatal_halt
    tbnz x0, #2, fatal_halt
    msr SPSel, #1
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:  cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:  ldr x0, =__stack_top
    mov sp, x0
    adrp x0, vector_table
    add x0, x0, :lo12:vector_table
    tst x0, #0x7ff
    b.ne fatal_halt
    msr VBAR_EL1, x0
    isb
    bl rust_main
fatal_halt:
    wfe
    b fatal_halt

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
    b irq_handler
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
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128

    .section .text.handler, "ax"
irq_handler:
    // Frame = x0..x30, interrupted SP, ELR_EL1, SPSR_EL1; 272 bytes.
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
    add x0, sp, #272
    str x0, [sp, #248]
    mrs x0, ELR_EL1
    str x0, [sp, #256]
    mrs x0, SPSR_EL1
    str x0, [sp, #264]
    // Accept only virtual-timer PPI 27 on this specific GICv2 VM.
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
    // Current frame is retained on the interrupted task's own stack.
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    ldr x2, [x0]
    cmp x2, #2
    b.hi irq_fatal
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    mov x4, sp
    str x4, [x3, x2, lsl #3]
    // Append actual interrupted actor, counter, ELR, SPSR, SP and selection.
    adrp x5, TICKS
    add x5, x5, :lo12:TICKS
    ldr x6, [x5]
    cmp x6, #8
    b.hs irq_fatal
    adrp x7, EVENTS
    add x7, x7, :lo12:EVENTS
    mov x8, #72
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
    mov x8, #3
    str x8, [x7, #40] // Sentinel: no selected task until selection succeeds.
    add x6, x6, #1
    str x6, [x5]
    cmp x6, #8
    b.eq all_done
    // The fixed synthetic test script uses ticks 1 and 2 to block A and B,
    // tick 3 to wake A while idle runs, and tick 4 to wake B. x19..x21
    // are scratch here: interrupted values were saved in the exception frame.
    mov x19, x2
    mov x20, x6
    mov x21, #0
    cmp x20, #1
    b.eq block_a
    cmp x20, #2
    b.eq block_b
    cmp x20, #3
    b.eq wake_a
    cmp x20, #4
    b.eq wake_b
    b requeue_current
block_a:
    cmp x19, #0
    b.ne irq_fatal
    mov x21, #1
    b block_current
block_b:
    cmp x19, #1
    b.ne irq_fatal
    mov x21, #2
block_current:
    adrp x4, TASK_STATE
    add x4, x4, :lo12:TASK_STATE
    ldr x5, [x4, x19, lsl #3]
    cmp x5, #2
    b.ne irq_fatal
    mov x5, #3
    str x5, [x4, x19, lsl #3]
    b choose_next
wake_a:
    cmp x19, #2
    b.ne irq_fatal
    mov x21, #3
    mov x0, #0
    bl wake_blocked
    b choose_next
wake_b:
    cmp x19, #0
    b.ne irq_fatal
    mov x21, #4
    mov x0, #1
    bl wake_blocked
requeue_current:
    cmp x19, #1
    b.hi irq_fatal
    adrp x4, TASK_STATE
    add x4, x4, :lo12:TASK_STATE
    ldr x5, [x4, x19, lsl #3]
    cmp x5, #2
    b.ne irq_fatal
    mov x5, #1
    str x5, [x4, x19, lsl #3]
    mov x0, x19
    bl enqueue_ready
choose_next:
    bl select_ready
    cmp x0, #2
    b.hi irq_fatal
    // ID 2 is idle; the real task queue must be empty and neither task running.
    cmp x0, #2
    b.ne selected_valid
    adrp x4, TASK_STATE
    add x4, x4, :lo12:TASK_STATE
    ldp x5, x6, [x4]
    cmp x5, #3
    b.ne irq_fatal
    cmp x6, #3
    b.ne irq_fatal
selected_valid:
    // Record the returned ID rather than inferring it from the prior actor.
    adrp x9, TICKS
    add x9, x9, :lo12:TICKS
    ldr x9, [x9]
    sub x9, x9, #1
    adrp x10, EVENTS
    add x10, x10, :lo12:EVENTS
    mov x11, #72
    madd x10, x9, x11, x10
    str x0, [x10, #40]
    mov x2, x0
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    str x2, [x0]
    mov x0, x21
    bl record_snapshot
    // The one-shot timer is rearmed before returning to the selected task.
    adrp x5, DELTA
    add x5, x5, :lo12:DELTA
    ldr x5, [x5]
    msr CNTV_TVAL_EL0, x5
    mov x5, #1
    msr CNTV_CTL_EL0, x5
    isb
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    adrp x2, CURRENT
    add x2, x2, :lo12:CURRENT
    ldr x2, [x2]
    ldr x0, [x3, x2, lsl #3]
    b restore_frame
all_done:
    // Timer remains disabled; enter a separate non-returning finish stack.
    mov x0, #5
    bl record_snapshot
    adrp x0, DONE_FRAME
    add x0, x0, :lo12:DONE_FRAME
    b restore_frame

    // x0 = task ID already marked READY. Append to the fixed ring, rejecting
    // invalid IDs, duplicate entries, invalid head and overflow.
enqueue_ready:
    cmp x0, #2
    b.hs irq_fatal
    adrp x1, TASK_STATE
    add x1, x1, :lo12:TASK_STATE
    ldr x2, [x1, x0, lsl #3]
    cmp x2, #1
    b.ne irq_fatal
    adrp x3, READY_LEN
    add x3, x3, :lo12:READY_LEN
    ldr x4, [x3]
    cmp x4, #1
    b.hi irq_fatal
    adrp x5, READY_HEAD
    add x5, x5, :lo12:READY_HEAD
    ldr x6, [x5]
    cmp x6, #1
    b.hi irq_fatal
    adrp x7, READY_QUEUE
    add x7, x7, :lo12:READY_QUEUE
    cbz x4, enqueue_store
    ldr x8, [x7, x6, lsl #3]
    cmp x8, x0
    b.eq irq_fatal
enqueue_store:
    add x6, x6, x4
    and x6, x6, #1
    str x0, [x7, x6, lsl #3]
    add x4, x4, #1
    str x4, [x3]
    ret

    // x0 = blocked task ID. Transition BLOCKED -> READY and enqueue.
wake_blocked:
    cmp x0, #2
    b.hs irq_fatal
    adrp x1, TASK_STATE
    add x1, x1, :lo12:TASK_STATE
    ldr x2, [x1, x0, lsl #3]
    cmp x2, #3
    b.ne irq_fatal
    mov x2, #1
    str x2, [x1, x0, lsl #3]
    b enqueue_ready

    // Record state and queue after a transition; x0 identifies the synthetic
    // action. This is a bounded in-memory journal, never serial I/O.
record_snapshot:
    adrp x1, TICKS
    add x1, x1, :lo12:TICKS
    ldr x2, [x1]
    sub x2, x2, #1
    cmp x2, #8
    b.hs irq_fatal
    adrp x3, EVENTS
    add x3, x3, :lo12:EVENTS
    mov x4, #72
    madd x3, x2, x4, x3
    adrp x5, TASK_STATE
    add x5, x5, :lo12:TASK_STATE
    ldp x6, x7, [x5]
    cmp x6, #3
    b.hi irq_fatal
    cmp x7, #3
    b.hi irq_fatal
    orr x6, x6, x7, lsl #8
    str x6, [x3, #48]
    adrp x5, READY_LEN
    add x5, x5, :lo12:READY_LEN
    ldr x6, [x5]
    cmp x6, #2
    b.hi irq_fatal
    str x6, [x3, #56]
    str x0, [x3, #64]
    ret

    // Pop a READY task from the ring. Return 2 when empty; on malformed
    // state stop without UART output, including when called from IRQ context.
    .global select_ready
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
1:  wfe
    b 1b

    .global start_first
start_first:
    // x0 points at a complete initial frame; does not return to caller.
restore_frame:
    mov x30, x0
    ldr x1, [x30, #256]
    msr ELR_EL1, x1
    ldr x1, [x30, #264]
    msr SPSR_EL1, x1
    ldr x1, [x30, #248]
    mov sp, x1
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

    .section .text.tasks, "ax"
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
    .global idle_loop
    .global idle_end
idle_loop:
    wfi
    b idle_loop
idle_end:
"#);

const STACK_BYTES: usize = 16 * 1024;
#[repr(C, align(16))]
struct Stack([u8; STACK_BYTES]);
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Frame { regs: [u64;31], sp:u64, elr:u64, spsr:u64 }
#[repr(C)]
#[derive(Clone, Copy)]
struct Event { actor:u64, count:u64, pc:u64, spsr:u64, sp:u64, next:u64, states:u64, ready:u64, action:u64 }
const EMPTY: Frame = Frame { regs:[0;31], sp:0, elr:0, spsr:0 };
static mut A_STACK:Stack=Stack([0;STACK_BYTES]);
static mut B_STACK:Stack=Stack([0;STACK_BYTES]);
static mut IDLE_STACK:Stack=Stack([0;STACK_BYTES]);
static mut DONE_STACK:Stack=Stack([0;STACK_BYTES]);
static mut INITIAL_A:Frame=EMPTY;
static mut INITIAL_B:Frame=EMPTY;
static mut INITIAL_IDLE:Frame=EMPTY;
#[no_mangle] static mut DONE_FRAME:Frame=EMPTY;
#[no_mangle] static mut SAVED:[u64;3]=[0;3];
#[no_mangle] static mut CURRENT:u64=0;
// 0 = inactive, 1 = ready, 2 = running, 3 = blocked. ID 2 is idle.
#[no_mangle] static mut TASK_STATE:[u64;2]=[0;2];
#[no_mangle] static mut READY_QUEUE:[u64;2]=[0;2];
#[no_mangle] static mut READY_HEAD:u64=0;
#[no_mangle] static mut READY_LEN:u64=0;
#[no_mangle] static mut TICKS:u64=0;
#[no_mangle] static mut DELTA:u64=0;
#[no_mangle] static mut EVENTS:[Event;8]=[Event { actor:0,count:0,pc:0,spsr:0,sp:0,next:3,states:0,ready:0,action:0 };8];
extern "C" { fn start_first(frame:*const Frame)->!; fn select_ready()->u64; static task_a_loop:u8; static task_a_end:u8; static task_b_loop:u8; static task_b_end:u8; static idle_loop:u8; static idle_end:u8; static vector_table:u8; }
const GICD:usize=0x0800_0000; const GICC:usize=0x0801_0000;
fn write(s:&str) { for b in s.bytes() { unsafe { while (0x0900_0018 as *const u32).read_volatile() & (1<<5)!=0 {core::hint::spin_loop()} (0x0900_0000 as *mut u32).write_volatile(b as u32); } } }
fn decimal(mut n:u64) { let mut digits=[0u8;20]; let mut i=20; loop { i-=1; digits[i]=b'0'+(n%10) as u8; n/=10; if n==0 {break} } for &b in &digits[i..] { unsafe {while (0x0900_0018 as *const u32).read_volatile()&(1<<5)!=0 {core::hint::spin_loop()} (0x0900_0000 as *mut u32).write_volatile(b as u32);} } }
fn physical_address(value:usize)->usize { let v:usize; unsafe {asm!("mov {v}, {input}",v=out(reg) v,input=in(reg) value,options(nomem,nostack,preserves_flags));} v }
fn halt()->! {loop {unsafe {asm!("wfe",options(nomem,nostack))}}}
fn check(ok:bool,msg:&str) {if !ok {write("FAIL ");write(msg);write("\r\n");halt()}}
fn frame(actor:u64,sp:usize,pc:usize)->Frame {
    let mut f=EMPTY;
    for i in 0..31 {f.regs[i]=1000+actor*100+i as u64;}
    f.regs[19]=0; f.sp=sp as u64; f.elr=pc as u64;
    // EL1h; IRQ unmasked, D/A/F masked. Tasks have distinct NZCV.
    f.spsr=if actor==1 {0x5000_0345} else {0xa000_0345}; f
}
#[no_mangle]
pub extern "C" fn rust_main()->! {
    // Before tasks start, an empty ready queue returns the idle sentinel.
    let empty=unsafe {select_ready()};
    check(empty==2 && unsafe {read_volatile(addr_of!(READY_LEN))}==0
          && unsafe {read_volatile(addr_of!(READY_HEAD))}==0
          && unsafe {read_volatile(addr_of!(TASK_STATE).cast::<u64>())}==0
          && unsafe {read_volatile(addr_of!(TASK_STATE).cast::<u64>().add(1))}==0, "empty ready queue");
    write("CHECK empty_ready=idle_sentinel\r\n");
    let a=physical_address(addr_of!(A_STACK) as usize); let b=physical_address(addr_of!(B_STACK) as usize);
    let idle=physical_address(addr_of!(IDLE_STACK) as usize); let d=physical_address(addr_of!(DONE_STACK) as usize);
    let stacks=[a,b,idle,d];
    for i in 0..stacks.len() {
        check(stacks[i]%16==0,"stack alignment");
        for j in 0..i { check(stacks[i]+STACK_BYTES<=stacks[j] || stacks[j]+STACK_BYTES<=stacks[i],"stack overlap"); }
    }
    check(core::mem::size_of::<Frame>()==272 && core::mem::size_of::<Event>()==72,"frame or event layout");
    let mut freq:u64; let mut vbar:u64;
    unsafe {asm!("mrs {}, CNTFRQ_EL0",out(reg) freq); asm!("mrs {}, VBAR_EL1",out(reg) vbar);}
    check(vbar == addr_of!(vector_table) as u64 && vbar&0x7ff==0,"vector");
    let delta=freq/100; check(delta>0 && delta<=i32::MAX as u64,"timer frequency");
    unsafe {
        addr_of_mut!(INITIAL_A).write(frame(0,a+STACK_BYTES,addr_of!(task_a_loop) as usize));
        addr_of_mut!(INITIAL_B).write(frame(1,b+STACK_BYTES,addr_of!(task_b_loop) as usize));
        addr_of_mut!(INITIAL_IDLE).write(frame(2,idle+STACK_BYTES,addr_of!(idle_loop) as usize));
        let mut done=frame(3,d+STACK_BYTES,finish as *const () as usize);
        done.spsr=0xa000_03c5; // keep IRQ masked during final checks
        addr_of_mut!(DONE_FRAME).write(done);
        addr_of_mut!(SAVED).cast::<u64>().write(addr_of!(INITIAL_A) as u64);
        addr_of_mut!(SAVED).cast::<u64>().add(1).write(addr_of!(INITIAL_B) as u64);
        addr_of_mut!(SAVED).cast::<u64>().add(2).write(addr_of!(INITIAL_IDLE) as u64);
        addr_of_mut!(DELTA).write(delta);
        addr_of_mut!(TASK_STATE).cast::<u64>().write(2);
        addr_of_mut!(TASK_STATE).cast::<u64>().add(1).write(1);
        addr_of_mut!(READY_QUEUE).cast::<u64>().write(1);
        addr_of_mut!(READY_HEAD).write(0);
        addr_of_mut!(READY_LEN).write(1);
        write_volatile((GICD+0x000) as *mut u32,0); write_volatile((GICC+0x000) as *mut u32,0);
        write_volatile((GICD+0x180) as *mut u32,1<<27); write_volatile((GICD+0x280) as *mut u32,1<<27);
        write_volatile((GICD+0x400+27) as *mut u8,0x80); write_volatile((GICC+0x004) as *mut u32,0xff);
        write_volatile((GICD+0x100) as *mut u32,1<<27); write_volatile((GICD+0x000) as *mut u32,1); write_volatile((GICC+0x000) as *mut u32,1);
        asm!("dsb sy","isb");
        write("BOOT qemu-gicv2 timer=virtual-ppi27 tasks=2 idle=wfi yields=0\r\n");
        asm!("msr CNTV_TVAL_EL0, {}", in(reg) delta);
        asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);
        asm!("isb");
        start_first(addr_of!(INITIAL_A));
    }
}
#[no_mangle]
extern "C" fn finish()->! {
    let ticks=unsafe {read_volatile(addr_of!(TICKS))}; check(ticks==8,"tick count");
    let actors=[0u64,1,2,0,1,0,1,0];
    let next=[1u64,2,0,1,0,1,0,3];
    let states=[0x203u64,0x303,0x302,0x201,0x102,0x201,0x102,0x102];
    let ready=[0u64,0,0,1,1,1,1,1];
    let actions=[1u64,2,3,4,0,0,0,5];
    let mut previous=[0u64;3];
    for i in 0..8 {
        let e=unsafe {read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};
        let expected=actors[i];
        check(e.actor==expected && e.next==next[i] && e.states==states[i] && e.ready==ready[i] && e.action==actions[i],"event transition or queue");
        if expected==2 { check(e.count==0,"idle counter"); }
        else { check(e.count>previous[expected as usize],"task did not progress"); previous[expected as usize]=e.count; }
        let (start,end)=match expected {
            0=>(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64),
            1=>(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64),
            _=>(addr_of!(idle_loop) as u64,addr_of!(idle_end) as u64),
        };
        check(e.pc>=start && e.pc<end && e.pc%4==0,"interrupted PC");
        let expected_spsr=if expected==1 {0x5000_0345} else {0xa000_0345};
        let stack_base=physical_address(match expected {0=>addr_of!(A_STACK) as usize,1=>addr_of!(B_STACK) as usize,_=>addr_of!(IDLE_STACK) as usize});
        check(e.spsr & 0xf000_03ff == expected_spsr && e.sp as usize==stack_base+STACK_BYTES,"per IRQ SPSR or SP");
        write("IRQ tick=");decimal(i as u64+1);write(" task=");decimal(e.actor);write(" next=");
        if e.next==3 {write("finish")} else {decimal(e.next)}
        write(" action=");decimal(e.action);write(" states=");decimal(e.states);write(" ready=");decimal(e.ready);write(" count=");decimal(e.count);write("\r\n");
    }
    for actor in 0..3usize {
        let ptr=unsafe {read_volatile(addr_of!(SAVED).cast::<u64>().add(actor))} as *const Frame;
        let base=physical_address(match actor {0=>addr_of!(A_STACK) as usize,1=>addr_of!(B_STACK) as usize,_=>addr_of!(IDLE_STACK) as usize});
        let frame_addr=ptr as usize;
        let Some(frame_end)=frame_addr.checked_add(272) else {check(false,"frame address overflow"); halt()};
        let Some(stack_end)=base.checked_add(STACK_BYTES) else {check(false,"stack address overflow"); halt()};
        check(frame_addr%16==0 && frame_addr>=base && frame_end<=stack_end,"frame outside stack");
        let f=unsafe {read_volatile(ptr)};
        check(f.sp as usize==frame_end && f.sp as usize==stack_end,"interrupted stack");
        let expected_spsr=if actor==1 {0x5000_0345} else {0xa000_0345};
        check(f.spsr & 0xf000_03ff == expected_spsr,"saved SPSR");
        for i in 0..31 { if i!=19 {check(f.regs[i]==1000+actor as u64*100+i as u64,"task GPR corrupted")} }
        check(f.regs[19]==previous[actor],"counter frame mismatch");
    }
    let mut ctl:u64; unsafe {asm!("mrs {}, CNTV_CTL_EL0",out(reg) ctl)};
    let active=unsafe {read_volatile((GICD+0x300) as *const u32)};
    check(ctl&1==0 && active&(1<<27)==0,"timer or GIC active");
    let current=unsafe {read_volatile(addr_of!(CURRENT))} as usize;
    let head=unsafe {read_volatile(addr_of!(READY_HEAD))} as usize;
    let len=unsafe {read_volatile(addr_of!(READY_LEN))};
    check(current==0 && head<2 && len==1,"terminal queue shape");
    let queued=unsafe {read_volatile(addr_of!(READY_QUEUE).cast::<u64>().add(head))};
    let a_state=unsafe {read_volatile(addr_of!(TASK_STATE).cast::<u64>())};
    let b_state=unsafe {read_volatile(addr_of!(TASK_STATE).cast::<u64>().add(1))};
    check(queued==1 && a_state==2 && b_state==1,"terminal queue states");
    write("CHECK blocked_a_b=observed idle_context=observed wake_a_b=observed states_and_queue=verified\r\n");
    write("CHECK task_and_idle_frames=verified timer=disabled gic=inactive terminal_snapshot=residual\r\n");
    write("PASS preemptive block wake idle scenario\r\n"); halt()
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>)->! {write("FAIL kernel panic\r\n");halt()}
