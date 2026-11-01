//! Two immutable EL0 roots, shared kernel mappings, QEMU identity DMA only.
use core::{arch::asm, ptr::{addr_of,addr_of_mut,read_volatile,write_volatile}};
const PAGE:usize=4096;const RAM_BASE:usize=0x4000_0000;const RAM_END:usize=RAM_BASE+128*1024*1024;
const UART:usize=0x0900_0000;const VIRTIO:usize=0x0a00_0000;const GICD:usize=0x0800_0000;const GICC:usize=0x0801_0000;
pub const ALIAS:usize=0x5000_0000;
const AF:u64=1<<10;const SH:u64=3<<8;const RO:u64=1<<7;const USER:u64=1<<6;const NORMAL:u64=1<<2;
const NG:u64=1<<11;const PXN:u64=1<<53;const UXN:u64=1<<54;const DESC:u64=3;const PA_MASK:u64=0x0000_ffff_ffff_f000;
#[repr(C,align(4096))] struct Table([u64;512]);
static mut L0_A:Table=Table([0;512]);static mut L0_B:Table=Table([0;512]);
static mut L1_A:Table=Table([0;512]);static mut L1_B:Table=Table([0;512]);
static mut L2_LOW:Table=Table([0;512]);static mut L2_RAM_A:Table=Table([0;512]);static mut L2_RAM_B:Table=Table([0;512]);
static mut L3_UART:Table=Table([0;512]);static mut L3_VIRTIO:Table=Table([0;512]);static mut L3_GIC:Table=Table([0;512]);
static mut L3_ALIAS_A:Table=Table([0;512]);static mut L3_ALIAS_B:Table=Table([0;512]);
static mut L3_RAM_A:[Table;64]=[const {Table([0;512])};64];
static mut L3_RAM_B:[Table;64]=[const {Table([0;512])};64];
#[no_mangle] static mut ROOTS:[u64;2]=[0;2];
extern "C" {static __image_start:u8;static __image_end:u8;static __text_end:u8;static __rodata_end:u8;static __data_start:u8;static __bss_end:u8;static __stack_guard:u8;static __stack_bottom:u8;static __stack_top:u8;static __usertext_start:u8;static __usertext_split:u8;static __usertext_end:u8;static __userstack_start:u8;static __userstack_end:u8;}
fn observed(v:usize)->usize {let r:usize;unsafe{asm!("mov {r}, {v}",r=out(reg) r,v=in(reg) v,options(nomem,nostack,preserves_flags));}r}
fn symbol(p:*const u8)->usize {observed(p as usize)}
fn table_addr(p:*mut Table,start:usize,end:usize)->u64 {let a=observed(p as usize);assert!(a%PAGE==0 && a>=start && a.checked_add(PAGE).is_some_and(|v|v<=end));a as u64}
unsafe fn set(p:*mut Table,i:usize,v:u64) {assert!(i<512);p.cast::<u64>().add(i).write_volatile(v);}
fn query(address:usize,write:bool,user:bool)->u64 {
 let par:u64;unsafe {
  if user && write {asm!("at s1e0w, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
  else if user {asm!("at s1e0r, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
  else if write {asm!("at s1e1w, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
  else {asm!("at s1e1r, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
 } par
}
pub fn root(index:usize)->u64 {assert!(index<2);unsafe{read_volatile(addr_of!(ROOTS).cast::<u64>().add(index))}}
pub fn active_root()->u64 {let v:u64;unsafe{asm!("mrs {}, TTBR0_EL1",out(reg) v)}v}
pub fn switch_root(root:u64) {unsafe{asm!("dsb ishst","msr TTBR0_EL1, {r}","isb","tlbi vmalle1","dsb ish","isb",r=in(reg) root);}}
/// Remove an exited task's private-page alias before its frame is recycled.
/// Caller must have stopped the task and masked IRQs. This fixed two-root
/// guest has a single CPU; this is not a general SMP address-space teardown.
pub fn unmap_task_alias(actor:usize,expected:usize) {
 assert!(actor<2 && active_root()==root(0));
 let table=if actor==0 {addr_of_mut!(L3_ALIAS_A)}else{addr_of_mut!(L3_ALIAS_B)};
 let entry=unsafe{table.cast::<u64>().add((ALIAS>>12)&511)};
 let old=unsafe{read_volatile(entry)};
 assert!(old&3==DESC && old&PA_MASK==(expected as u64&PA_MASK));
 unsafe{write_volatile(entry,0);asm!("dsb ishst","tlbi vmalle1","dsb ish","isb");}
 switch_root(root(actor));
 assert!(query(ALIAS,false,true)&1!=0 && query(ALIAS,true,true)&1!=0);
 switch_root(root(0));
}
pub fn identity_range(start:usize,len:usize)->bool {
 if len==0{return false} let Some(end)=start.checked_add(len)else{return false};let mut p=start&!(PAGE-1);
 while p<end {let result=query(p,false,false);if result&1!=0 || (result&PA_MASK)!=(p as u64&PA_MASK){return false}let Some(next)=p.checked_add(PAGE)else{return false};p=next;}true
}
pub fn shareable_range(start:usize,len:usize)->bool {
 let data=symbol(addr_of!(__data_start));let end=symbol(addr_of!(__stack_top));
 len>0 && start>=data && start.checked_add(len).is_some_and(|v|v<=end) && identity_range(start,len)
}
pub fn init(heap:usize,heap_len:usize,dma:usize,dma_len:usize,a:usize,b:usize) {
 let heap=observed(heap);let dma=observed(dma);let a=observed(a);let b=observed(b);
 let start=symbol(addr_of!(__image_start));let end=symbol(addr_of!(__image_end));let text_end=symbol(addr_of!(__text_end));let ro_end=symbol(addr_of!(__rodata_end));let data=symbol(addr_of!(__data_start));
 let bss_end=symbol(addr_of!(__bss_end));let guard=symbol(addr_of!(__stack_guard));let stack_bottom=symbol(addr_of!(__stack_bottom));let stack_top=symbol(addr_of!(__stack_top));let ut0=symbol(addr_of!(__usertext_start));let split=symbol(addr_of!(__usertext_split));let ut1=symbol(addr_of!(__usertext_end));let us0=symbol(addr_of!(__userstack_start));let us1=symbol(addr_of!(__userstack_end));
 let mut sp:usize;let mut sctlr:u64;unsafe{asm!("mov {},sp",out(reg) sp);asm!("mrs {},SCTLR_EL1",out(reg) sctlr);}
 assert!(sctlr&((1<<0)|(1<<2)|(1<<12)|(1<<25))==0);
 assert!(start>=RAM_BASE && end<=RAM_END && start%PAGE==0 && end%PAGE==0 && start<text_end && text_end<=ut0 && ut0<ut1 && ut1<=ro_end && ro_end==data && data<us0 && us0<us1 && us1==end);
 assert!([ut0,ut1,us0,us1,a,b].iter().all(|x|x%PAGE==0) && ut0.checked_add(PAGE)==Some(split) && split.checked_add(PAGE)==Some(ut1) && us0.checked_add(4*PAGE)==Some(us1));
 assert!(a>=us0 && b>=us0 && a.checked_add(PAGE).is_some_and(|v|v<=us1) && b.checked_add(PAGE).is_some_and(|v|v<=us1) && (a+PAGE<=b || b+PAGE<=a));
 assert!(guard==bss_end && guard%PAGE==0 && guard.checked_add(PAGE)==Some(stack_bottom) && stack_bottom.checked_add(0x40000)==Some(stack_top));
 assert!(sp>stack_bottom && sp<=stack_top && stack_top<=us0 && sp%16==0);
 let heap_end=heap.checked_add(heap_len).unwrap();let dma_end=dma.checked_add(dma_len).unwrap();
 assert!(heap%PAGE==0 && dma%PAGE==0 && heap>=data && dma>=data && heap_end<=bss_end && dma_end<=bss_end && (heap_end<=dma || dma_end<=heap));
 let tables=[addr_of_mut!(L0_A),addr_of_mut!(L0_B),addr_of_mut!(L1_A),addr_of_mut!(L1_B),addr_of_mut!(L2_LOW),addr_of_mut!(L2_RAM_A),addr_of_mut!(L2_RAM_B),addr_of_mut!(L3_UART),addr_of_mut!(L3_VIRTIO),addr_of_mut!(L3_GIC),addr_of_mut!(L3_ALIAS_A),addr_of_mut!(L3_ALIAS_B)];
 let ram_a=addr_of_mut!(L3_RAM_A).cast::<Table>();let ram_b=addr_of_mut!(L3_RAM_B).cast::<Table>();let mut addrs=[0u64;140];
 for i in 0..12 {addrs[i]=table_addr(tables[i],data,bss_end);}
 for i in 0..64 {addrs[12+i]=table_addr(unsafe{ram_a.add(i)},data,bss_end);addrs[76+i]=table_addr(unsafe{ram_b.add(i)},data,bss_end);}
 for i in 0..140 {let v=addrs[i] as usize;assert!(v+PAGE<=heap || heap_end<=v);assert!(v+PAGE<=dma || dma_end<=v);for j in 0..i {assert!(addrs[i]!=addrs[j]);}}
 let roots=[addrs[0]|(1u64<<48),addrs[1]|(2u64<<48)];
 unsafe {
  write_volatile(addr_of_mut!(ROOTS).cast::<u64>(),roots[0]);write_volatile(addr_of_mut!(ROOTS).cast::<u64>().add(1),roots[1]);
  set(tables[0],0,addrs[2]|DESC);set(tables[1],0,addrs[3]|DESC);
  set(tables[2],0,addrs[4]|DESC);set(tables[3],0,addrs[4]|DESC);
  set(tables[2],1,addrs[5]|DESC);set(tables[3],1,addrs[6]|DESC);
  set(tables[4],(UART>>21)&511,addrs[7]|DESC);set(tables[4],(VIRTIO>>21)&511,addrs[8]|DESC);set(tables[4],(GICD>>21)&511,addrs[9]|DESC);
  set(tables[7],(UART>>12)&511,UART as u64|AF|PXN|UXN|DESC);
  for i in 0..4 {set(tables[8],((VIRTIO>>12)&511)+i,(VIRTIO+i*PAGE) as u64|AF|PXN|UXN|DESC);}
  set(tables[9],(GICD>>12)&511,GICD as u64|AF|PXN|UXN|DESC);set(tables[9],(GICC>>12)&511,GICC as u64|AF|PXN|UXN|DESC);
  for i in 0..64 {set(tables[5],i,addrs[12+i]|DESC);set(tables[6],i,addrs[76+i]|DESC);}
  set(tables[5],(ALIAS>>21)&511,addrs[10]|DESC);set(tables[6],(ALIAS>>21)&511,addrs[11]|DESC);
  set(tables[10],(ALIAS>>12)&511,a as u64|AF|SH|NORMAL|USER|PXN|UXN|NG|DESC);
  set(tables[11],(ALIAS>>12)&511,b as u64|AF|SH|NORMAL|USER|PXN|UXN|NG|DESC);
  for address in (start..end).step_by(PAGE) {
   // Keep the entire guard unmapped in both roots. The stack grows down.
   if address==guard {continue}
   let index=(address-RAM_BASE)/(2*1024*1024);let entry=(address>>12)&511;
   let kernel_flags=if address<text_end {AF|SH|NORMAL|RO|UXN}
    else if address>=ut0 && address<ut1 {AF|SH|NORMAL|RO|PXN|UXN|NG}
    else if address<ro_end {AF|SH|NORMAL|RO|PXN|UXN}
    else {AF|SH|NORMAL|PXN|UXN};
   let user_code=AF|SH|NORMAL|RO|USER|PXN|NG;
   let a_flags=if address>=ut0 && address<split {user_code}else{kernel_flags};
   let b_flags=if address>=split && address<ut1 {user_code}else{kernel_flags};
   set(ram_a.add(index),entry,address as u64|a_flags|DESC);
   set(ram_b.add(index),entry,address as u64|b_flags|DESC);
  }
  let mair=0x4400u64;let tcr=16u64|(3<<12)|(1<<23);
  asm!("dsb sy","msr MAIR_EL1, {mair}","msr TCR_EL1, {tcr}","msr TTBR0_EL1, {root}","isb","tlbi vmalle1","dsb sy","isb","mrs x9, SCTLR_EL1","orr x9, x9, #1","msr SCTLR_EL1, x9","isb",mair=in(reg) mair,tcr=in(reg) tcr,root=in(reg) roots[0],out("x9") _);
  asm!("mrs {}, SCTLR_EL1",out(reg) sctlr);
 }
 assert!(sctlr&1!=0 && sctlr&((1<<2)|(1<<12))==0);
 for (i,&root) in roots.iter().enumerate() {
  switch_root(root);assert!(active_root()==root);
  let alias=query(ALIAS,false,true);let expected=if i==0 {a} else {b};
  assert!(alias&1==0 && alias&PA_MASK==expected as u64 && query(ALIAS,true,true)&1==0 && query(ALIAS+PAGE,false,true)&1!=0);
  assert!(identity_range(start,guard-start) && identity_range(stack_bottom,end-stack_bottom) && identity_range(heap,heap_len) && identity_range(dma,dma_len));
  assert!(query(guard,false,false)&1!=0 && query(guard,true,false)&1!=0 && query(guard+PAGE-1,false,false)&1!=0);
  assert!(identity_range(UART,PAGE) && identity_range(VIRTIO,4*PAGE) && identity_range(GICD,PAGE) && identity_range(GICC,PAGE));
  assert!(query(start-PAGE,false,false)&1!=0 && query(end,false,false)&1!=0 && query(UART+PAGE,false,false)&1!=0 && query(VIRTIO+4*PAGE,false,false)&1!=0);
  assert!(query(start,true,false)&1!=0 && query(data,true,false)&1==0);
  for address in [data,stack_top-1,UART,VIRTIO,VIRTIO+4*PAGE-1,GICD,GICC,a,b] {assert!(query(address,false,true)&1!=0 && query(address,true,true)&1!=0);}
  let own=if i==0 {ut0}else{split};let other=if i==0 {split}else{ut0};
  assert!(query(own,false,true)&1==0 && query(own,true,true)&1!=0);
  assert!(query(other,false,true)&1!=0 && query(other,true,true)&1!=0);
  // Every per-ASID code mapping is non-global: even the inaccessible mapping
  // must not be cached as global when the other root grants EL0 access.
  let ram=if i==0 {ram_a}else{ram_b};
  let desc=|va:usize| unsafe {ram.add((va-RAM_BASE)/(2*1024*1024)).cast::<u64>().add((va>>12)&511).read_volatile()};
  let permission=RO|USER|PXN|UXN|NG;
  assert!(desc(own)&permission==RO|USER|PXN|NG);
  assert!(desc(other)&permission==RO|PXN|UXN|NG);
 }
 switch_root(roots[0]);assert!(!shareable_range(us0,PAGE));
 let dev=unsafe{tables[8].cast::<u64>().read_volatile()};let gc=unsafe{tables[9].cast::<u64>().add((GICD>>12)&511).read_volatile()};
 assert!((dev&(PXN|UXN))==(PXN|UXN) && (dev&(7<<2))==0 && (gc&(PXN|UXN))==(PXN|UXN) && (gc&(7<<2))==0);
}
