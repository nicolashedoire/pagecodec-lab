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
#[derive(Clone,Copy,PartialEq,Eq)]
struct Message { data:[u8;16], len:u8 }
const EMPTY_MSG:Message=Message {data:[0;16],len:0};
#[derive(Clone,Copy,PartialEq,Eq)]
struct Mailbox { slots:[Message;2], head:u8, len:u8 }
const EMPTY_BOX:Mailbox=Mailbox { slots:[EMPTY_MSG;2],head:0,len:0 };
#[derive(Clone,Copy,PartialEq,Eq)]
struct Stats { ok_send:u8, ok_recv:u8, invalid:u8, target:u8, size:u8, full:u8, empty:u8 }
const EMPTY_STATS:Stats=Stats {ok_send:0,ok_recv:0,invalid:0,target:0,size:0,full:0,empty:0};
struct State { inbox:[Mailbox;2], stats:[Stats;2], switches:u8 }
static mut STATE:State=State {inbox:[EMPTY_BOX;2],stats:[EMPTY_STATS;2],switches:0};
extern "C" {
    static __image_start:u8; static __image_end:u8; static __text_end:u8; static __rodata_end:u8;
    static __usertext_start:u8; static __usertext_end:u8; static __userstack_start:u8; static __userstack_end:u8;
    static __bss_end:u8; static __stack_top:u8; static vector_table:u8;
    static user_a:u8; static user_b:u8;
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
fn bump(n:&mut u8) { *n=n.saturating_add(1); }
fn in_range(p:u64,n:u64)->bool { n>0 && n<=16 && p>=ALIAS as u64 && p.checked_add(n).is_some_and(|end|end<=(ALIAS+PAGE) as u64) }
unsafe fn user_read(p:u64)->u8 { let v:u32; asm!("ldrb {v:w}, [{p}]",p=in(reg)p,v=out(reg)v,options(nostack,preserves_flags)); v as u8 }
unsafe fn user_write(p:u64,v:u8) { asm!("strb {v:w}, [{p}]",p=in(reg)p,v=in(reg)(v as u32),options(nostack,preserves_flags)); }
// Status: 0 success, 1 pointer/range invalid, 2 invalid size/capacity,
// 3 mailbox full, 4 mailbox empty, 5 invalid destination.
#[no_mangle]
unsafe extern "C" fn svc_dispatch(f:*mut Frame,esr:u64)->u64 {
    let f=&mut *f; let actor=read_volatile(addr_of!(CURRENT));
    let mut ttbr:u64; asm!("mrs {}, TTBR0_EL1",out(reg) ttbr);
    let ut0=physical(addr_of!(__usertext_start) as usize) as u64;
    let ut1=physical(addr_of!(__usertext_end) as usize) as u64;
    if actor>1 || ttbr!=read_volatile(addr_of!(ROOTS).cast::<u64>().add(actor as usize)) || esr>>26!=0x15 || esr&(1<<25)==0 || f.sp!=(ALIAS+PAGE) as u64 || f.spsr&0x3cf!=0x3c0 || f.elr<ut0 || f.elr>=ut1 || f.elr%4!=0 { return 4; }
    let a=actor as usize;let st=&mut *addr_of_mut!(STATE);
    let before=st.inbox; let p=f.regs[0];let n=f.regs[1];let imm=esr&0xffff;
    match imm {
      0x10 => {
        let dest=f.regs[2];
        let status=if dest>1 || dest==actor {5} else if n==0 || n>16 {2} else if !in_range(p,n) {1} else if st.inbox[dest as usize].len==2 {3} else {
            let q=&mut st.inbox[dest as usize]; let idx=(q.head as usize+q.len as usize)%2;
            let mut msg=EMPTY_MSG; for i in 0..n as usize {msg.data[i]=user_read(p+i as u64);} msg.len=n as u8;
            q.slots[idx]=msg; q.len+=1; bump(&mut st.stats[a].ok_send); 0
        };
        if status!=0 {if st.inbox!=before {return 4;} match status {1=>bump(&mut st.stats[a].invalid),2=>bump(&mut st.stats[a].size),3=>bump(&mut st.stats[a].full),5=>bump(&mut st.stats[a].target),_=>{}}}
        f.regs[0]=status; f.regs[1]=0; 0
      },
      0x11 => {
        let status=if n==0 || n>16 {2} else if !in_range(p,n) {1} else if st.inbox[a].len==0 {4} else if st.inbox[a].slots[st.inbox[a].head as usize].len as u64>n {2} else {
          let q=&mut st.inbox[a];let idx=q.head as usize;let msg=q.slots[idx];
          for i in 0..msg.len as usize {user_write(p+i as u64,msg.data[i]);}
          q.slots[idx]=EMPTY_MSG;q.head=(q.head+1)%2;q.len-=1;f.regs[1]=msg.len as u64;bump(&mut st.stats[a].ok_recv);0
        };
        if status!=0 {if st.inbox!=before {return 4;} f.regs[1]=0;match status {1=>bump(&mut st.stats[a].invalid),2=>bump(&mut st.stats[a].size),4=>bump(&mut st.stats[a].empty),_=>{}}}
        f.regs[0]=status;0
      },
      0x12 => {bump(&mut st.switches);f.regs[0]=0;if actor==0 {1} else {2}},
      0x13 => if actor==0 {3} else {4},
      _=>4
    }
}
#[no_mangle] pub extern "C" fn guest_fatal()->! {write("FAIL unexpected exception or service\r\n");halt()}

#[no_mangle]
pub extern "C" fn rust_main()->! {
    let mut probe=254u8;bump(&mut probe);bump(&mut probe);check(probe==255,"saturating counters");
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
    check(st.switches==10 && st.inbox[0].len==0 && st.inbox[1].len==0 && st.inbox[0].slots==[EMPTY_MSG;2] && st.inbox[1].slots==[EMPTY_MSG;2],"empty mailboxes");
    check(st.stats[0]==Stats {ok_send:10,ok_recv:2,invalid:35,target:1,size:13,full:69,empty:6} && st.stats[1]==Stats {ok_send:2,ok_recv:10,invalid:0,target:0,size:1,full:1,empty:5},"service counters");
    let a=addr_of!(A_STACK).cast::<u8>(); let b=addr_of!(B_STACK).cast::<u8>();
    for i in 0..PAGE {
      let av=if i<5 {b"HELLO"[i]} else if (16..20).contains(&i){b"PING"[i-16]} else if (64..66).contains(&i){b"OK"[i-64]} else if (80..82).contains(&i){b"GO"[i-80]} else if i==128 {b'D'} else if i==129 {b'd'} else {0};
      let bv=if i<2 {b"OK"[i]} else if (16..18).contains(&i){b"GO"[i-16]} else if i==64 {b'D'} else if (65..69).contains(&i){b"HELLO"[i-64]} else if i==80 {b'd'} else if (81..84).contains(&i){b"PING"[i-80]} else {0};
      check(unsafe{read_volatile(a.add(i))}==av && unsafe{read_volatile(b.add(i))}==bv,"page contents");
    }
    write("CHECK counters=saturating_at_255\r\n");
    write("CHECK requests=a:HELLO,PING b:OK,GO fifo=preserved\r\n");
    write("REJECT pointer=page_cross,overflow,kernel size=17,short_buffer target=invalid mailbox_unchanged=yes\r\n");
    write("REJECT full=a,b empty=a,a,b mailboxes_unchanged=yes\r\n");
    write("CHECK generated_rounds=4 pointer_cases=36 size_cases=76 oracle=matched fifo_wrap=matched reject_mailboxes=unchanged\r\n");
    write("CHECK a_send=10 a_receive=2 b_send=2 b_receive=10 switches=10 pages=expected\r\n");
    write("PASS EL0 bounded IPC dispatcher\r\n");halt()
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>)->! {write("FAIL kernel panic\r\n");halt()}
