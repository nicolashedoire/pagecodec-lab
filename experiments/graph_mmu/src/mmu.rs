//! Narrow identity map for a single QEMU virt guest; not a general VM manager.
use core::{arch::asm, ptr::{addr_of, addr_of_mut}};
const PAGE:usize=4096;
const RAM_BASE:usize=0x4000_0000;
const RAM_END:usize=RAM_BASE+128*1024*1024;
const UART:usize=0x0900_0000;
const VIRTIO:usize=0x0a00_0000;
const AF:u64=1<<10;
const SH:u64=3<<8;
const RO:u64=1<<7;
const NORMAL:u64=1<<2;
const PXN:u64=1<<53;
const UXN:u64=1<<54;
const DESC:u64=3;
const PA_MASK:u64=0x0000_ffff_ffff_f000;
#[repr(C,align(4096))]
struct Table([u64;512]);
static mut L0:Table=Table([0;512]);
static mut L1:Table=Table([0;512]);
static mut L2_LOW:Table=Table([0;512]);
static mut L2_RAM:Table=Table([0;512]);
static mut L3_UART:Table=Table([0;512]);
static mut L3_VIRTIO:Table=Table([0;512]);
static mut L3_RAM:[Table;64]=[const {Table([0;512])};64];
extern "C" {static __image_start:u8;static __image_end:u8;static __text_end:u8;static __rodata_end:u8;static __data_start:u8;static __bss_end:u8;static __stack_top:u8;}

fn observed(v:usize)->usize {let r:usize;unsafe {asm!("mov {r}, {v}",r=out(reg) r,v=in(reg) v,options(nomem,nostack,preserves_flags));}r}
fn symbol(p:*const u8)->usize {observed(p as usize)}
fn table_addr(p:*mut Table,start:usize,end:usize)->u64 {
    let a=observed(p as usize);
    assert!(a%PAGE==0 && a>=start && a.checked_add(PAGE).is_some_and(|v|v<=end));a as u64
}
unsafe fn set(p:*mut Table,i:usize,v:u64) {assert!(i<512);p.cast::<u64>().add(i).write_volatile(v);}
fn query(address:usize,write:bool,user:bool)->u64 {
    let par:u64;
    unsafe {
        if user && write {asm!("at s1e0w, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
        else if user {asm!("at s1e0r, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
        else if write {asm!("at s1e1w, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
        else {asm!("at s1e1r, {a}","isb","mrs {p}, PAR_EL1",a=in(reg) address,p=out(reg) par);}
    } par
}
pub fn identity_range(start:usize,len:usize)->bool {
    if len==0 {return false}
    let Some(end)=start.checked_add(len) else {return false};
    let mut p=start & !(PAGE-1);
    while p<end {
        let result=query(p,false,false);
        if result&1!=0 || (result&PA_MASK)!=(p as u64&PA_MASK) {return false}
        let Some(next)=p.checked_add(PAGE) else {return false};p=next;
    }
    true
}
/// Only the identity-mapped, writable image region may be shared with the QEMU device.
pub fn shareable_range(start:usize,len:usize)->bool {
    let data=symbol(addr_of!(__data_start));let end=symbol(addr_of!(__image_end));
    len>0 && start>=data && start.checked_add(len).is_some_and(|v|v<=end) && identity_range(start,len)
}
/// Initializes all page tables before switching SCTLR.M. No allocations or IRQs.
pub fn init(heap:usize,heap_len:usize,dma:usize,dma_len:usize) {
    let start=symbol(addr_of!(__image_start)); let end=symbol(addr_of!(__image_end));
    let text_end=symbol(addr_of!(__text_end));let ro_end=symbol(addr_of!(__rodata_end));let data=symbol(addr_of!(__data_start));
    let bss_end=symbol(addr_of!(__bss_end));let stack_top=symbol(addr_of!(__stack_top));
    let mut sp:usize;let mut sctlr:u64;
    unsafe {asm!("mov {}, sp",out(reg) sp);asm!("mrs {}, SCTLR_EL1",out(reg) sctlr);}
    assert!(sctlr & ((1<<0)|(1<<2)|(1<<12)|(1<<25))==0);
    assert!(start>=RAM_BASE && end<=RAM_END && start%PAGE==0 && end%PAGE==0 && start<text_end && text_end<=ro_end && ro_end==data && data<end && text_end%PAGE==0 && ro_end%PAGE==0);
    assert!(sp> bss_end && sp<=stack_top && stack_top<=end && sp%16==0);
    let heap_end=heap.checked_add(heap_len).unwrap();let dma_end=dma.checked_add(dma_len).unwrap();
    assert!(heap%PAGE==0 && dma%PAGE==0 && heap>=data && dma>=data && heap_end<=bss_end && dma_end<=bss_end && (heap_end<=dma || dma_end<=heap));
    let l0=addr_of_mut!(L0);let l1=addr_of_mut!(L1);let low=addr_of_mut!(L2_LOW);let ram=addr_of_mut!(L2_RAM);
    let uart=addr_of_mut!(L3_UART);let virtio=addr_of_mut!(L3_VIRTIO);let tables=addr_of_mut!(L3_RAM).cast::<Table>();
    let a0=table_addr(l0,data,bss_end);let a1=table_addr(l1,data,bss_end);let alow=table_addr(low,data,bss_end);let aram=table_addr(ram,data,bss_end);
    let auart=table_addr(uart,data,bss_end);let avirtio=table_addr(virtio,data,bss_end);
    let mut addrs=[0u64;70];addrs[..6].copy_from_slice(&[a0,a1,alow,aram,auart,avirtio]);
    for i in 0..64 {addrs[6+i]=table_addr(unsafe {tables.add(i)},data,bss_end);}
    for i in 0..70 {
        let a=addrs[i] as usize;
        assert!(a+PAGE<=heap || heap_end<=a);assert!(a+PAGE<=dma || dma_end<=a);
        for j in 0..i {assert!(addrs[i]!=addrs[j]);}
    }
    unsafe {
        set(l0,0,a1|DESC);set(l1,0,alow|DESC);set(l1,1,aram|DESC);
        set(low,(UART>>21)&511,auart|DESC);set(low,(VIRTIO>>21)&511,avirtio|DESC);
        set(uart,(UART>>12)&511,UART as u64|AF|PXN|UXN|DESC);
        for i in 0..4 {set(virtio,((VIRTIO>>12)&511)+i,(VIRTIO+i*PAGE) as u64|AF|PXN|UXN|DESC);}
        for i in 0..64 {set(ram,i,addrs[6+i]|DESC);}
        for address in (start..end).step_by(PAGE) {
            let flags=if address<text_end {AF|SH|NORMAL|RO|UXN}
                else if address<ro_end {AF|SH|NORMAL|RO|PXN|UXN}
                else {AF|SH|NORMAL|PXN|UXN};
            let t=tables.add((address-RAM_BASE)/(2*1024*1024));
            set(t,(address>>12)&511,address as u64|flags|DESC);
        }
        let mair=0x4400u64;let tcr=16u64|(3<<12)|(1<<23);
        asm!("dsb sy","msr MAIR_EL1, {mair}","msr TCR_EL1, {tcr}","msr TTBR0_EL1, {root}","isb","tlbi vmalle1","dsb sy","isb","mrs x9, SCTLR_EL1","orr x9, x9, #1","msr SCTLR_EL1, x9","isb",mair=in(reg) mair,tcr=in(reg) tcr,root=in(reg) a0,out("x9") _);
        asm!("mrs {}, SCTLR_EL1",out(reg) sctlr);
    }
    assert!(sctlr&1!=0 && sctlr&((1<<2)|(1<<12))==0);
    assert!(identity_range(start,end-start) && identity_range(heap,heap_len) && identity_range(dma,dma_len));
    assert!(identity_range(UART,PAGE) && identity_range(VIRTIO,4*PAGE));
    assert!(query(start-PAGE,false,false)&1!=0 && query(end,false,false)&1!=0 && query(UART+PAGE,false,false)&1!=0 && query(VIRTIO+4*PAGE,false,false)&1!=0);
    assert!(query(start,true,false)&1!=0 && query(data,true,false)&1==0);
    for address in [data,end-1,UART,VIRTIO,VIRTIO+4*PAGE-1] {
        assert!(query(address,false,true)&1!=0 && query(address,true,true)&1!=0);
    }
    let data_table=unsafe {tables.add((data-RAM_BASE)/(2*1024*1024))};
    let data_desc=unsafe {data_table.cast::<u64>().add((data>>12)&511).read_volatile()};
    let device_desc=unsafe {virtio.cast::<u64>().read_volatile()};
    assert!((data_desc&(PXN|UXN))==(PXN|UXN) && (device_desc&(PXN|UXN))==(PXN|UXN) && (device_desc&(7<<2))==0);
}
