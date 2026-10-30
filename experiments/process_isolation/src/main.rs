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
    b.ne boot_halt
    mrs x0, SCTLR_EL1
    tbnz x0, #0, boot_halt
    tbnz x0, #2, boot_halt
    tbnz x0, #12, boot_halt
    tbnz x0, #25, boot_halt
    msr SPSel, #1
    ldr x0, =__bss_start
    ldr x1, =__bss_end
zero_bss:
    cmp x0, x1
    b.hs bss_done
    str xzr, [x0], #8
    b zero_bss
bss_done:
    ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
boot_halt:
    wfe
    b boot_halt
"#);

const PAGE:usize=4096;
const N:usize=512;
const RAM:usize=0x4000_0000;
const REGION:usize=2*1024*1024;
const UART:usize=0x0900_0000;
const GICD:usize=0x0800_0000;
const GICC:usize=0x0801_0000;
const ALIAS:usize=0x5000_0000;
const NONGLOBAL:u64=1<<11;
const UXN:u64=1<<54; const PXN:u64=1<<53; const AF:u64=1<<10;
const SH:u64=3<<8; const RO:u64=1<<7; const USER:u64=1<<6;
const NORMAL:u64=1<<2; const PAGE_DESC:u64=3;
#[repr(C,align(4096))] struct Table([u64;N]);
#[repr(C,align(4096))] struct UserStack([u8;PAGE]);
#[repr(C,align(16))] #[derive(Clone,Copy)]
struct Frame { regs:[u64;31], sp:u64, elr:u64, spsr:u64 }
#[repr(C)] #[derive(Clone,Copy)]
struct Event {actor:u64,count:u64,pc:u64,spsr:u64,sp:u64,next:u64,kernel_sp:u64,active_ttbr:u64,next_ttbr:u64}
const EMPTY:Frame=Frame {regs:[0;31],sp:0,elr:0,spsr:0};
static mut L0_A:Table=Table([0;N]);static mut L0_B:Table=Table([0;N]);
static mut L1_A:Table=Table([0;N]);static mut L1_B:Table=Table([0;N]);
static mut L2_LOW:Table=Table([0;N]);static mut L2_RAM_A:Table=Table([0;N]);static mut L2_RAM_B:Table=Table([0;N]);
static mut L3_UART:Table=Table([0;N]);static mut L3_GIC:Table=Table([0;N]);static mut L3_RAM:Table=Table([0;N]);
static mut L3_ALIAS_A:Table=Table([0;N]);static mut L3_ALIAS_B:Table=Table([0;N]);
#[no_mangle] #[link_section=".userstack"] static mut A_STACK:UserStack=UserStack([0;PAGE]);
#[no_mangle] #[link_section=".userstack"] static mut B_STACK:UserStack=UserStack([0;PAGE]);
static mut KERNEL_DATA:u64=0x0123_4567_89ab_cdef;
static mut A_FRAME:Frame=EMPTY; static mut B_FRAME:Frame=EMPTY;
#[no_mangle] static mut SAVED:[u64;2]=[0;2];
#[no_mangle] static mut ROOTS:[u64;2]=[0;2];
#[no_mangle] static mut CURRENT:u64=0;
#[no_mangle] static mut TASK_STATE:[u64;2]=[0;2];
#[no_mangle] static mut READY_QUEUE:[u64;2]=[0;2];
#[no_mangle] static mut READY_HEAD:u64=0;
#[no_mangle] static mut READY_LEN:u64=0;
#[no_mangle] static mut TICKS:u64=0;
#[no_mangle] static mut DELTA:u64=0;
#[no_mangle] static mut EVENTS:[Event;8]=[Event {actor:0,count:0,pc:0,spsr:0,sp:0,next:2,kernel_sp:0,active_ttbr:0,next_ttbr:0};8];
#[no_mangle] static mut FAULT_COUNT:[u64;2]=[0;2];
#[no_mangle] static mut FAULT_ESR:[u64;2]=[0;2];
#[no_mangle] static mut FAULT_FAR:[u64;2]=[0;2];
#[no_mangle] static mut FAULT_PC:[u64;2]=[0;2];
#[no_mangle] static mut FAULT_RESUME:[u64;2]=[0;2];
#[no_mangle] static mut FAULT_SPSR:[u64;2]=[0;2];
extern "C" {
    static __image_start:u8; static __image_end:u8; static __text_end:u8; static __rodata_end:u8;
    static __usertext_start:u8; static __usertext_end:u8; static __userstack_start:u8; static __userstack_end:u8;
    static __bss_end:u8; static __stack_top:u8; static vector_table:u8;
    static task_a_loop:u8; static task_a_end:u8; static task_b_loop:u8; static task_b_end:u8;
    static task_a_forbidden:u8; static task_b_forbidden:u8;
    fn start_first(frame:*const Frame)->!;
}
fn write(s:&str) {for b in s.bytes(){unsafe {while (0x0900_0018 as *const u32).read_volatile()&(1<<5)!=0 {core::hint::spin_loop()} (0x0900_0000 as *mut u32).write_volatile(b as u32);}}}
fn decimal(mut n:u64) {let mut digits=[0u8;20];let mut i=20;loop{i-=1;digits[i]=b'0'+(n%10) as u8;n/=10;if n==0{break}}for &b in &digits[i..]{unsafe{while (0x0900_0018 as *const u32).read_volatile()&(1<<5)!=0{core::hint::spin_loop()}(0x0900_0000 as *mut u32).write_volatile(b as u32);}}}
fn halt()->! {loop{unsafe{asm!("wfe",options(nomem,nostack))}}}
fn check(ok:bool,msg:&str){if !ok{write("FAIL ");write(msg);write("\r\n");halt()}}
fn physical(value:usize)->usize {let out:usize;unsafe{asm!("mov {out}, {value}",out=out(reg) out,value=in(reg) value,options(nomem,nostack,preserves_flags));}out}
fn table_addr(p:*mut Table,start:usize,end:usize)->u64 {let a=p as usize;check(a%PAGE==0 && a>=start && a.checked_add(PAGE).is_some_and(|n|n<=end),"table bounds");a as u64}
unsafe fn set(table:*mut Table,index:usize,value:u64) {assert!(index<N);table.cast::<u64>().add(index).write_volatile(value);}
fn query_user(address:usize,write:bool)->u64 {
    let par:u64;
    unsafe {
        if write {asm!("at s1e0w, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
        else {asm!("at s1e0r, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
    }
    par
}
fn switch_root(root:u64) {
    // Immutable tables, one CPU: use a conservative full local EL1 TLB
    // invalidation on every root change, with publication/ordering barriers.
    unsafe {asm!("dsb ishst","msr TTBR0_EL1, {r}","isb","tlbi vmalle1","dsb ish","isb",r=in(reg) root);}
}
fn frame(actor:u64,sp:usize,pc:usize,forbidden:usize)->Frame {
    let mut f=EMPTY;
    for i in 0..31 {f.regs[i]=1000+actor*100+i as u64;}
    f.regs[0]=ALIAS as u64;f.regs[1]=if actor==0 {0xa1a1} else {0xb2b2};f.regs[2]=0;f.regs[3]=0;f.regs[4]=0;
    f.regs[5]=0;f.regs[6]=0x777;f.regs[7]=forbidden as u64;f.regs[8]=0;
    f.regs[19]=0;f.sp=sp as u64;f.elr=pc as u64;
    // EL0t, IRQ unmasked, debug/SError/FIQ masked, different NZCV values.
    f.spsr=if actor==0 {0xa000_0340} else {0x5000_0340};f
}

#[no_mangle]
pub extern "C" fn rust_main()->! {
    let mut el:u64;let mut sctlr:u64;let mut sp:u64;
    unsafe {asm!("mrs {}, CurrentEL",out(reg) el);asm!("mrs {}, SCTLR_EL1",out(reg) sctlr);asm!("mov {}, sp",out(reg) sp);}
    let start=physical(addr_of!(__image_start) as usize);let end=physical(addr_of!(__image_end) as usize);
    let text_end=physical(addr_of!(__text_end) as usize);let ro_end=physical(addr_of!(__rodata_end) as usize);
    let ut0=physical(addr_of!(__usertext_start) as usize);let ut1=physical(addr_of!(__usertext_end) as usize);
    let us0=physical(addr_of!(__userstack_start) as usize);let us1=physical(addr_of!(__userstack_end) as usize);
    let a=physical(addr_of_mut!(A_STACK) as usize);let b=physical(addr_of_mut!(B_STACK) as usize);
    let kt=physical(addr_of!(__stack_top) as usize);let kb=physical(addr_of!(__bss_end) as usize);
    let vector=physical(addr_of!(vector_table) as usize);let protected=physical(addr_of_mut!(KERNEL_DATA) as usize);
    let a_pc=physical(addr_of!(task_a_loop) as usize);let a_end=physical(addr_of!(task_a_end) as usize);
    let b_pc=physical(addr_of!(task_b_loop) as usize);let b_end=physical(addr_of!(task_b_end) as usize);
    check(el==4 && sctlr&((1<<0)|(1<<2)|(1<<12)|(1<<25))==0 && sp%16==0,"boot state");
    check(start%PAGE==0 && end%PAGE==0 && start>=RAM && end<=RAM+REGION && start<text_end && text_end<=ut0 && ut0<ut1 && ut1<=ro_end && ro_end<us0 && us0<us1 && us1<=end,"image sections");
    check([text_end,ut0,ut1,ro_end,us0,us1,a,b,ALIAS].iter().all(|v|v%PAGE==0) && ut0.checked_add(PAGE)==Some(ut1) && us0.checked_add(2*PAGE)==Some(us1),"user pages");
    check(a>=us0 && b>=us0 && a.checked_add(PAGE).is_some_and(|x|x<=us1) && b.checked_add(PAGE).is_some_and(|x|x<=us1) && (a+PAGE<=b || b+PAGE<=a),"physical page separation");
    check(kb<kt && sp as usize>kb && sp as usize<=kt && kt<=us0 && protected>=ro_end && protected+8<=kb,"kernel stack and data");
    check(vector%2048==0 && vector>=start && vector.checked_add(2048).is_some_and(|x|x<=text_end),"vector");
    check(a_pc>=ut0 && a_pc<a_end && a_end<=ut1 && b_pc>=ut0 && b_pc<b_end && b_end<=ut1,"user entry ranges");
    check(core::mem::size_of::<Frame>()==272 && core::mem::size_of::<Event>()==72,"frame layout");
    for p in [addr_of_mut!(A_STACK).cast::<u8>(),addr_of_mut!(B_STACK).cast::<u8>()] {
        for i in 0..PAGE {unsafe{write_volatile(p.add(i),0)}}
        for i in 0..PAGE {check(unsafe{read_volatile(p.add(i))}==0,"page zero");}
    }
    unsafe {asm!("msr VBAR_EL1, {}","isb",in(reg) vector);}
    let mut vbar:u64;unsafe{asm!("mrs {}, VBAR_EL1",out(reg) vbar);}
    check(vbar==vector as u64,"VBAR readback");
    let tables=[addr_of_mut!(L0_A),addr_of_mut!(L0_B),addr_of_mut!(L1_A),addr_of_mut!(L1_B),addr_of_mut!(L2_LOW),addr_of_mut!(L2_RAM_A),addr_of_mut!(L2_RAM_B),addr_of_mut!(L3_UART),addr_of_mut!(L3_GIC),addr_of_mut!(L3_RAM),addr_of_mut!(L3_ALIAS_A),addr_of_mut!(L3_ALIAS_B)];
    let mut addrs=[0u64;12];for i in 0..tables.len(){addrs[i]=table_addr(tables[i],ro_end,kb);for j in 0..i{check(addrs[i]!=addrs[j],"table overlap");}}
    let roots=[addrs[0]|(1u64<<48),addrs[1]|(2u64<<48)];
    unsafe {
        addr_of_mut!(ROOTS).cast::<u64>().write(roots[0]);addr_of_mut!(ROOTS).cast::<u64>().add(1).write(roots[1]);
        set(tables[0],0,addrs[2]|PAGE_DESC);set(tables[1],0,addrs[3]|PAGE_DESC);
        set(tables[2],0,addrs[4]|PAGE_DESC);set(tables[3],0,addrs[4]|PAGE_DESC);
        set(tables[2],1,addrs[5]|PAGE_DESC);set(tables[3],1,addrs[6]|PAGE_DESC);
        set(tables[4],(UART>>21)&511,addrs[7]|PAGE_DESC);set(tables[4],(GICD>>21)&511,addrs[8]|PAGE_DESC);
        set(tables[5],(RAM>>21)&511,addrs[9]|PAGE_DESC);set(tables[6],(RAM>>21)&511,addrs[9]|PAGE_DESC);
        set(tables[5],(ALIAS>>21)&511,addrs[10]|PAGE_DESC);set(tables[6],(ALIAS>>21)&511,addrs[11]|PAGE_DESC);
        set(tables[7],(UART>>12)&511,UART as u64|AF|PXN|UXN|PAGE_DESC);
        set(tables[8],(GICD>>12)&511,GICD as u64|AF|PXN|UXN|PAGE_DESC);set(tables[8],(GICC>>12)&511,GICC as u64|AF|PXN|UXN|PAGE_DESC);
        set(tables[10],(ALIAS>>12)&511,a as u64|AF|SH|NORMAL|USER|PXN|UXN|NONGLOBAL|PAGE_DESC);
        set(tables[11],(ALIAS>>12)&511,b as u64|AF|SH|NORMAL|USER|PXN|UXN|NONGLOBAL|PAGE_DESC);
        for addr in (start..end).step_by(PAGE){
            let flags=if addr<text_end {AF|SH|NORMAL|RO|UXN}
                else if addr>=ut0 && addr<ut1 {AF|SH|NORMAL|RO|USER|PXN|NONGLOBAL}
                else if addr<ro_end {AF|SH|NORMAL|RO|PXN|UXN}
                else {AF|SH|NORMAL|PXN|UXN};
            set(tables[9],(addr>>12)&511,addr as u64|flags|PAGE_DESC);
        }
        let mair=0x4400u64;let tcr=16u64|(3<<12)|(1<<23);
        asm!("dsb sy","msr MAIR_EL1, {m}","msr TCR_EL1, {t}","msr TTBR0_EL1, {r}","isb","tlbi vmalle1","dsb sy","isb","mrs x9, SCTLR_EL1","orr x9, x9, #1","msr SCTLR_EL1, x9","isb",m=in(reg) mair,t=in(reg) tcr,r=in(reg) roots[0],out("x9") _);
    }
    unsafe{asm!("mrs {}, SCTLR_EL1",out(reg) sctlr);}check(sctlr&1!=0,"MMU enabled");
    for i in 0..2 {
        switch_root(roots[i]);
        let pa=query_user(ALIAS,false);
        let expected=if i==0 {a} else {b};
        check(pa&1==0 && pa&0x0000_ffff_ffff_f000==(expected as u64) && query_user(ALIAS,true)&1==0,"private alias translation");
        check(query_user(ALIAS+PAGE,false)&1!=0,"alias neighbor mapped");
        check(query_user(a,false)&1!=0 && query_user(b,false)&1!=0 && query_user(protected,false)&1!=0 && query_user(protected,true)&1!=0,"private physical or kernel access");
        check(query_user(UART,false)&1!=0 && query_user(GICD,false)&1!=0 && query_user(GICC,false)&1!=0 && query_user(ut0,false)&1==0 && query_user(ut0,true)&1!=0,"EL0 device or code permission");
    }
    switch_root(roots[0]);
    write("MMU alias=same_va physical_pages=distinct asids=1,2 kernel=privileged\r\n");
    let mut freq:u64;unsafe{asm!("mrs {}, CNTFRQ_EL0",out(reg) freq);}
    let delta=freq/100;check(delta>0 && delta<=i32::MAX as u64,"timer frequency");
    unsafe {
        addr_of_mut!(A_FRAME).write(frame(0,ALIAS+PAGE,a_pc,b));
        addr_of_mut!(B_FRAME).write(frame(1,ALIAS+PAGE,b_pc,a));
        addr_of_mut!(SAVED).cast::<u64>().write(addr_of!(A_FRAME) as u64);addr_of_mut!(SAVED).cast::<u64>().add(1).write(addr_of!(B_FRAME) as u64);
        addr_of_mut!(TASK_STATE).cast::<u64>().write(2);addr_of_mut!(TASK_STATE).cast::<u64>().add(1).write(1);
        addr_of_mut!(READY_QUEUE).cast::<u64>().write(1);addr_of_mut!(READY_HEAD).write(0);addr_of_mut!(READY_LEN).write(1);addr_of_mut!(DELTA).write(delta);
        write_volatile((GICD+0x000) as *mut u32,0);write_volatile((GICC+0x000) as *mut u32,0);
        write_volatile((GICD+0x180) as *mut u32,1<<27);write_volatile((GICD+0x280) as *mut u32,1<<27);
        write_volatile((GICD+0x400+27) as *mut u8,0x80);write_volatile((GICC+0x004) as *mut u32,0xff);
        write_volatile((GICD+0x100) as *mut u32,1<<27);write_volatile((GICD+0x000) as *mut u32,1);write_volatile((GICC+0x000) as *mut u32,1);
        asm!("dsb sy","isb");write("BOOT el0_spaces=2 alias=shared_va timer=ppi27 yields=0\r\n");
        asm!("msr CNTV_TVAL_EL0, {}",in(reg) delta);asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);asm!("isb");
        start_first(addr_of!(A_FRAME));
    }
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
    b lower_el_abort        // lower EL AArch64 sync
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
lower_el_abort:
    sub sp, sp, #256
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
    mrs x0, SPSR_EL1
    and x1, x0, #0xf
    cbnz x1, irq_fatal
    mrs x0, SP_EL0
    mov x1, #0x50000000
    add x1, x1, #1, lsl #12
    cmp x0, x1
    b.ne irq_fatal
    mrs x0, ESR_EL1
    tbz x0, #25, irq_fatal
    tbnz x0, #6, irq_fatal
    tbnz x0, #7, irq_fatal
    tbnz x0, #10, irq_fatal
    lsr x1, x0, #26
    cmp x1, #0x24
    b.ne irq_fatal
    and x1, x0, #0x3f
    cmp x1, #0x0f
    b.ne irq_fatal
    adrp x1, CURRENT
    add x1, x1, :lo12:CURRENT
    ldr x2, [x1]
    cmp x2, #1
    b.hi irq_fatal
    mrs x3, TTBR0_EL1
    adrp x4, ROOTS
    add x4, x4, :lo12:ROOTS
    ldr x4, [x4, x2, lsl #3]
    cmp x3, x4
    b.ne irq_fatal
    mrs x3, FAR_EL1
    cbnz x2, abort_b_target
    adrp x4, B_STACK
    add x4, x4, :lo12:B_STACK
    b abort_target_check
abort_b_target:
    adrp x4, A_STACK
    add x4, x4, :lo12:A_STACK
abort_target_check:
    cmp x3, x4
    b.ne irq_fatal
    ldr x5, [sp, #56]
    cmp x5, x4
    b.ne irq_fatal
    mrs x5, ELR_EL1
    cbnz x2, abort_b_site
    adrp x6, task_a_forbidden
    add x6, x6, :lo12:task_a_forbidden
    b abort_site_check
abort_b_site:
    adrp x6, task_b_forbidden
    add x6, x6, :lo12:task_b_forbidden
abort_site_check:
    cmp x5, x6
    b.ne irq_fatal
    adrp x4, FAULT_COUNT
    add x4, x4, :lo12:FAULT_COUNT
    ldr x6, [x4, x2, lsl #3]
    cbnz x6, irq_fatal
    mov x6, #1
    str x6, [x4, x2, lsl #3]
    adrp x4, FAULT_ESR
    add x4, x4, :lo12:FAULT_ESR
    str x0, [x4, x2, lsl #3]
    adrp x4, FAULT_FAR
    add x4, x4, :lo12:FAULT_FAR
    str x3, [x4, x2, lsl #3]
    adrp x4, FAULT_PC
    add x4, x4, :lo12:FAULT_PC
    str x5, [x4, x2, lsl #3]
    mrs x6, SPSR_EL1
    adrp x4, FAULT_SPSR
    add x4, x4, :lo12:FAULT_SPSR
    str x6, [x4, x2, lsl #3]
    add x5, x5, #4
    adrp x4, FAULT_RESUME
    add x4, x4, :lo12:FAULT_RESUME
    str x5, [x4, x2, lsl #3]
    msr ELR_EL1, x5
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    add sp, sp, #256
    eret

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
    mov x8, #2
    str x8, [x7, #40]
    add x8, sp, #272
    str x8, [x7, #48]
    mrs x8, TTBR0_EL1
    str x8, [x7, #56]
    str xzr, [x7, #64]
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
    mov x11, #72
    madd x10, x9, x11, x10
    str x0, [x10, #40]
    mov x2, x0
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    str x2, [x0]
    // Single CPU, immutable tables. Publish the new root/ASID and invalidate
    // local EL1/EL0 translations before restoring that actor's EL0 frame.
    adrp x4, ROOTS
    add x4, x4, :lo12:ROOTS
    ldr x5, [x4, x2, lsl #3]
    dsb ishst
    msr TTBR0_EL1, x5
    isb
    tlbi vmalle1
    dsb ish
    isb
    mrs x6, TTBR0_EL1
    str x6, [x10, #64]
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
    // Timer disabled and IRQ masked on exception entry. Finish in EL1.
    b finish

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

    .section .usertext, "ax"
    .global task_a_loop
    .global task_a_end
task_a_loop:
    cbnz x5, task_a_after_probe
    mov x5, #1
    .global task_a_forbidden
task_a_forbidden:
    ldr x6, [x7]
    mov x8, #0x777
    eor x8, x6, x8
    cbnz x8, irq_fatal
task_a_after_probe:
    ldr x2, [x0]
    cbnz x4, 4f
    cbz x2, 3f
    eor x3, x2, x1
    cbnz x3, irq_fatal
3:  str x1, [x0]
    mov x4, #1
    b 5f
4:  eor x3, x2, x1
    cbnz x3, irq_fatal
5:  str x1, [x0]
    add x19, x19, #1
    b task_a_loop
task_a_end:
    .global task_b_loop
    .global task_b_end
task_b_loop:
    cbnz x5, task_b_after_probe
    mov x5, #1
    .global task_b_forbidden
task_b_forbidden:
    ldr x6, [x7]
    mov x8, #0x777
    eor x8, x6, x8
    cbnz x8, irq_fatal
task_b_after_probe:
    ldr x2, [x0]
    cbnz x4, 7f
    cbz x2, 6f
    eor x3, x2, x1
    cbnz x3, irq_fatal
6:  str x1, [x0]
    mov x4, #1
    b 8f
7:  eor x3, x2, x1
    cbnz x3, irq_fatal
8:  str x1, [x0]
    add x19, x19, #1
    b task_b_loop
task_b_end:
"#);

#[no_mangle]
pub extern "C" fn finish()->! {
    check(unsafe{read_volatile(addr_of!(TICKS))}==8,"tick count");
    let kb=physical(addr_of!(__bss_end) as usize);let kt=physical(addr_of!(__stack_top) as usize);
    let a=physical(addr_of!(A_STACK) as usize);let b=physical(addr_of!(B_STACK) as usize);
    let roots=[unsafe{read_volatile(addr_of!(ROOTS).cast::<u64>())},unsafe{read_volatile(addr_of!(ROOTS).cast::<u64>().add(1))}];
    let mut previous=[0u64;2];
    for i in 0..8 {
        let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};let actor=(i%2) as u64;
        let (lo,hi)=if actor==0 {(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64)} else {(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64)};
        let expected_spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        let expected_next=if i<7 {1-actor} else {2};
        let expected_root=if i<7 {roots[expected_next as usize]} else {0};
        check(e.actor==actor && e.next==expected_next && e.active_ttbr==roots[actor as usize] && e.next_ttbr==expected_root,"selected actor or TTBR");
        check(e.count>previous[actor as usize],"task did not progress");previous[actor as usize]=e.count;
        check(e.pc>=lo && e.pc<hi && e.pc%4==0,"user PC");
        check(e.spsr&0xf000_03ff==expected_spsr && e.sp as usize==ALIAS+PAGE,"user SPSR or SP");
        check(e.kernel_sp%16==0 && e.kernel_sp as usize>kb && (e.kernel_sp as usize)<=kt && (e.kernel_sp as usize).checked_sub(272).is_some_and(|n|n>=kb),"kernel IRQ stack");
        write("IRQ tick=");decimal(i as u64+1);write(" el0_task=");decimal(e.actor);write(" count=");decimal(e.count);write(" next=");
        if e.next<2 {decimal(e.next)} else {write("finish")};write(" active_asid=");decimal(e.active_ttbr>>48);write("\r\n");
    }
    for actor in 0..2usize {
        let saved=unsafe{read_volatile(addr_of!(SAVED).cast::<u64>().add(actor))} as *const Frame;
        let expected=if actor==0 {addr_of!(A_FRAME)} else {addr_of!(B_FRAME)};
        check(saved==expected,"frame pointer");let f=unsafe{read_volatile(expected)};
        let spsr=if actor==0 {0xa000_0340} else {0x5000_0340};let value=if actor==0 {0xa1a1} else {0xb2b2};
        check(f.sp as usize==ALIAS+PAGE && f.spsr&0xf000_03ff==spsr,"final user frame");
        check(f.regs[0]==ALIAS as u64 && f.regs[1]==value && f.regs[2]==value && f.regs[3]==0 && f.regs[4]==1,"user data registers");
        let forbidden=if actor==0 {b} else {a};
        check(f.regs[5]==1 && f.regs[6]==0x777 && f.regs[7]==forbidden as u64 && f.regs[8]==0,"negative probe registers");
        for i in 9..31 {if i!=19 {check(f.regs[i]==1000+actor as u64*100+i as u64,"GPR corruption")}}
        check(f.regs[19]==previous[actor],"counter mismatch");
    }
    for actor in 0..2usize {
        let count=unsafe{read_volatile(addr_of!(FAULT_COUNT).cast::<u64>().add(actor))};
        let esr=unsafe{read_volatile(addr_of!(FAULT_ESR).cast::<u64>().add(actor))};
        let far=unsafe{read_volatile(addr_of!(FAULT_FAR).cast::<u64>().add(actor))};
        let pc=unsafe{read_volatile(addr_of!(FAULT_PC).cast::<u64>().add(actor))};
        let resume=unsafe{read_volatile(addr_of!(FAULT_RESUME).cast::<u64>().add(actor))};
        let spsr=unsafe{read_volatile(addr_of!(FAULT_SPSR).cast::<u64>().add(actor))};
        let expected_far=if actor==0 {b} else {a};
        let expected_pc=if actor==0 {addr_of!(task_a_forbidden)} else {addr_of!(task_b_forbidden)};
        let expected_spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(count==1 && esr>>26==0x24 && esr&(1<<25)!=0 && esr&(1<<6)==0 && esr&(1<<7)==0 && esr&(1<<10)==0 && esr&0x3f==0x0f,"negative fault syndrome");
        check(far==expected_far as u64 && pc==expected_pc as u64 && pc.checked_add(4)==Some(resume) && spsr&0xf000_03ff==expected_spsr,"negative fault context");
    }
    write("FAULTS cross_identity_reads=denied a=1 b=1 ec=0x24 dfsc=0x0f elr=site_plus_4\r\n");
    for (p,expected) in [(addr_of!(A_STACK).cast::<u8>(),0xa1a1u64),(addr_of!(B_STACK).cast::<u8>(),0xb2b2u64)] {
        check(unsafe{read_volatile(p.cast::<u64>())}==expected,"private page value");
        for i in 8..PAGE {check(unsafe{read_volatile(p.add(i))}==0,"private page neighbor");}
    }
    write("CONTROL private_alias_writes_reads=ok values=a1a1,b2b2 neighboring_bytes=zero\r\n");
    for (root,pa) in [(roots[0],a),(roots[1],b)] {
        switch_root(root);let observed=query_user(ALIAS,false);
        check(observed&1==0 && observed&0x0000_ffff_ffff_f000==pa as u64,"final alias translation");
        check(query_user(a,false)&1!=0 && query_user(b,false)&1!=0,"private identity access");
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
    write("CHECK same_va=two_physical_pages values=a1a1,b2b2 neighbors=zero ttbr_asids=verified\r\n");
    write("CHECK private_identity_and_kernel=el0_at_denied el0_frames=verified timer=disabled gic=inactive\r\n");
    write("PASS cross-page isolation scenario in QEMU\r\n");halt();
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>)->! {write("FAIL kernel panic\r\n");halt()}
