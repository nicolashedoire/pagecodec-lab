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
const EMPTY:Frame=Frame {regs:[0;31],sp:0,elr:0,spsr:0};
static mut L0_A:Table=Table([0;N]);static mut L0_B:Table=Table([0;N]);
static mut L1_A:Table=Table([0;N]);static mut L1_B:Table=Table([0;N]);
static mut L2_LOW:Table=Table([0;N]);static mut L2_RAM_A:Table=Table([0;N]);static mut L2_RAM_B:Table=Table([0;N]);
static mut L3_UART:Table=Table([0;N]);static mut L3_GIC:Table=Table([0;N]);static mut L3_RAM:Table=Table([0;N]);
static mut L3_ALIAS_A:Table=Table([0;N]);static mut L3_ALIAS_B:Table=Table([0;N]);
#[link_section=".userstack"] static mut A_STACK:UserStack=UserStack([0;PAGE]);
#[link_section=".userstack"] static mut B_STACK:UserStack=UserStack([0;PAGE]);
static mut KERNEL_DATA:u64=0x0123_4567_89ab_cdef;
#[no_mangle] static mut A_FRAME:Frame=EMPTY;
#[no_mangle] static mut B_FRAME:Frame=EMPTY;
#[no_mangle] static mut ROOTS:[u64;2]=[0;2];
#[no_mangle] static mut CURRENT:u64=0;
struct State { phase:u8, bad_pointer:u8, bad_overflow:u8, bad_length:u8, msg_len:u8, reply_len:u8, msg:[u8;16], reply:[u8;16] }
static mut STATE:State=State {phase:0,bad_pointer:0,bad_overflow:0,bad_length:0,msg_len:0,reply_len:0,msg:[0;16],reply:[0;16]};
extern "C" {
    static __image_start:u8; static __image_end:u8; static __text_end:u8; static __rodata_end:u8;
    static __usertext_start:u8; static __usertext_end:u8; static __userstack_start:u8; static __userstack_end:u8;
    static __bss_end:u8; static __stack_top:u8; static vector_table:u8;
    static user_a:u8; static user_b:u8;
    static a_after_badptr:u8; static a_after_overflow:u8; static a_after_badlen:u8; static a_after_send:u8;
    static a_after_reply:u8; static a_after_finish:u8; static b_after_receive:u8; static b_after_reply:u8;
    fn start_first(frame:*const Frame)->!;
}
fn write(s:&str) {for b in s.bytes(){unsafe {while (0x0900_0018 as *const u32).read_volatile()&(1<<5)!=0 {core::hint::spin_loop()} (0x0900_0000 as *mut u32).write_volatile(b as u32);}}}
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
fn frame(pc:usize)->Frame { Frame {regs:[0;31],sp:(ALIAS+PAGE) as u64,elr:pc as u64,spsr:0x3c0} }
fn in_range(p:u64,n:u64)->bool { n>0 && n<=16 && p>=ALIAS as u64 && p.checked_add(n).is_some_and(|end|end<=(ALIAS+PAGE) as u64) }
unsafe fn user_read(p:u64)->u8 { let v:u32; asm!("ldrb {v:w}, [{p}]",p=in(reg)p,v=out(reg)v,options(nostack,preserves_flags)); v as u8 }
unsafe fn user_write(p:u64,v:u8) { asm!("strb {v:w}, [{p}]",p=in(reg)p,v=in(reg)(v as u32),options(nostack,preserves_flags)); }
#[no_mangle]
unsafe extern "C" fn svc_dispatch(f:*mut Frame,esr:u64)->u64 {
    let f=&mut *f; let actor=read_volatile(addr_of!(CURRENT));
    let mut ttbr:u64; asm!("mrs {}, TTBR0_EL1",out(reg) ttbr);
    if actor>1 || ttbr!=read_volatile(addr_of!(ROOTS).cast::<u64>().add(actor as usize)) || esr>>26!=0x15 || esr&(1<<25)==0 || f.sp!=(ALIAS+PAGE) as u64 || f.spsr&0x3cf!=0x3c0 { return 4; }
    let st=&mut *addr_of_mut!(STATE); let imm=esr&0xffff;
    let pc=match st.phase {0=>addr_of!(a_after_badptr),1=>addr_of!(a_after_overflow),2=>addr_of!(a_after_badlen),3=>addr_of!(a_after_send),4=>addr_of!(b_after_receive),5=>addr_of!(b_after_reply),6=>addr_of!(a_after_reply),7=>addr_of!(a_after_finish),_=>return 4};
    if f.elr!=physical(pc as usize) as u64 {return 4;}
    let p=f.regs[0];let n=f.regs[1];
    match (st.phase,actor,imm) {
      (0,0,0x10) if p==(ALIAS+PAGE-2) as u64 && n==5 && !in_range(p,n) && st.msg_len==0 && st.reply_len==0 && st.msg==[0;16] && st.reply==[0;16] => {st.bad_pointer=1;st.phase=1;f.regs[0]=1;0},
      (1,0,0x10) if p==u64::MAX-1 && n==5 && !in_range(p,n) && st.msg_len==0 && st.reply_len==0 && st.msg==[0;16] && st.reply==[0;16] => {st.bad_overflow=1;st.phase=2;f.regs[0]=1;0},
      (2,0,0x10) if p==ALIAS as u64 && n==17 && st.msg_len==0 && st.reply_len==0 && st.msg==[0;16] && st.reply==[0;16] => {st.bad_length=1;st.phase=3;f.regs[0]=2;0},
      (3,0,0x10) if p==ALIAS as u64 && n==5 && in_range(p,n) => {for i in 0..5 {st.msg[i]=user_read(p+i as u64);} if &st.msg[..5]!=b"HELLO" {return 4;} st.msg_len=5;st.phase=4;f.regs[0]=0;1},
      (4,1,0x11) if p==(ALIAS+32) as u64 && n==16 && in_range(p,n) && st.msg_len==5 => {for i in 0..5 {user_write(p+i as u64,st.msg[i]);} st.phase=5;f.regs[0]=0;f.regs[1]=5;0},
      (5,1,0x12) if p==(ALIAS+64) as u64 && n==2 && in_range(p,n) => {for i in 0..2 {st.reply[i]=user_read(p+i as u64);} if &st.reply[..2]!=b"OK" {return 4;} st.reply_len=2;st.phase=6;2},
      (6,0,0x13) if p==(ALIAS+32) as u64 && n==16 && in_range(p,n) && st.reply_len==2 => {for i in 0..2 {user_write(p+i as u64,st.reply[i]);} st.phase=7;f.regs[0]=0;f.regs[1]=2;0},
      (7,0,0x14) if p==0xbeef => {st.phase=8;3},
      _=>4
    }
}
#[no_mangle] pub extern "C" fn guest_fatal()->! {write("FAIL unexpected exception or service\r\n");halt()}

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
    let a_pc=physical(addr_of!(user_a) as usize);let b_pc=physical(addr_of!(user_b) as usize);
    check(el==4 && sctlr&((1<<0)|(1<<2)|(1<<12)|(1<<25))==0 && sp%16==0,"boot state");
    check(start%PAGE==0 && end%PAGE==0 && start>=RAM && end<=RAM+REGION && start<text_end && text_end<=ut0 && ut0<ut1 && ut1<=ro_end && ro_end<us0 && us0<us1 && us1<=end,"image sections");
    check([text_end,ut0,ut1,ro_end,us0,us1,a,b,ALIAS].iter().all(|v|v%PAGE==0) && ut0.checked_add(PAGE)==Some(ut1) && us0.checked_add(2*PAGE)==Some(us1),"user pages");
    check(a>=us0 && b>=us0 && a.checked_add(PAGE).is_some_and(|x|x<=us1) && b.checked_add(PAGE).is_some_and(|x|x<=us1) && (a+PAGE<=b || b+PAGE<=a),"physical page separation");
    check(kb<kt && sp as usize>kb && sp as usize<=kt && kt<=us0 && protected>=ro_end && protected+8<=kb,"kernel stack and data");
    check(vector%2048==0 && vector>=start && vector.checked_add(2048).is_some_and(|x|x<=text_end),"vector");
    check(a_pc>=ut0 && a_pc<ut1 && b_pc>=ut0 && b_pc<ut1,"user entry ranges");
    check(core::mem::size_of::<Frame>()==272,"frame layout");
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
    unsafe {addr_of_mut!(A_FRAME).write(frame(a_pc));addr_of_mut!(B_FRAME).write(frame(b_pc)); start_first(addr_of!(A_FRAME));}
}

global_asm!(include_str!("svc.S"));
#[no_mangle] pub extern "C" fn finish()->! {
    let st=unsafe{&*addr_of!(STATE)};
    check(st.phase==8 && st.bad_pointer==1 && st.bad_overflow==1 && st.bad_length==1 && st.msg_len==5 && st.reply_len==2 && &st.msg[..5]==b"HELLO" && &st.reply[..2]==b"OK", "ipc state");
    let a=addr_of!(A_STACK).cast::<u8>();let b=addr_of!(B_STACK).cast::<u8>();
    for i in 0..PAGE {
        let av=if i<5 {b"HELLO"[i]} else if (32..34).contains(&i){b"OK"[i-32]} else {0};
        let bv=if (32..37).contains(&i){b"HELLO"[i-32]} else if (64..66).contains(&i){b"OK"[i-64]} else {0};
        check(unsafe{read_volatile(a.add(i))}==av && unsafe{read_volatile(b.add(i))}==bv,"page contents");
    }
    write("REJECT range=crosses_page mailbox_unchanged=yes\r\n");
    write("REJECT range=integer_overflow mailbox_unchanged=yes\r\n");
    write("REJECT length=17 maximum=16 mailbox_unchanged=yes\r\n");
    write("SEND actor=a bytes=5 copied=HELLO\r\n");
    write("RECEIVE actor=b bytes=5 contents=HELLO private_page=yes\r\n");
    write("REPLY actor=b bytes=2 copied=OK\r\n");
    write("RECEIVE actor=a bytes=2 contents=OK\r\n");
    write("CHECK svc_sequence=8 kernel_buffers=bounded pages=expected\r\n");
    write("PASS EL0 bounded SVC message exchange\r\n"); halt()
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>)->! {write("FAIL kernel panic\r\n");halt()}
