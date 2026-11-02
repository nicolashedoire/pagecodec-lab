//! Independent ELF actors exercise private, recyclable pages and heap I/O.
#![no_std]
#![no_main]
extern crate alloc;
#[path="../guest_api.rs"] mod guest_api;
use alloc::vec::Vec;
use core::{alloc::{GlobalAlloc,Layout},arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
const HEAP:u64=0x50004000;
unsafe fn alloc()->u64 {let mut x=0u64;asm!("svc #0x180",inout("x0") x,options(nostack));x}
unsafe fn free(address:u64)->u64 {let mut x=address;asm!("svc #0x181",inout("x0") x,options(nostack));x}
// Fixture-only allocator: exactly one allocation backed by this task's page.
// It does not split blocks, grow, or provide a general-purpose process heap.
struct PageAllocator;
unsafe impl GlobalAlloc for PageAllocator {
 unsafe fn alloc(&self,layout:Layout)->*mut u8 {
  if layout.size()>4096 || layout.align()>4096 {return core::ptr::null_mut()}
  if alloc()==HEAP {HEAP as *mut u8} else {core::ptr::null_mut()}
 }
 unsafe fn dealloc(&self,ptr:*mut u8,_layout:Layout) {
  if ptr as u64!=HEAP || free(HEAP)!=0 {stop(42)}
 }
}
#[global_allocator] static ALLOCATOR:PageAllocator=PageAllocator;
unsafe fn io(write:bool,name:*mut u8,value:*mut u8)->u64 {
 let mut x=name as u64;
 if write {asm!("svc #0x141",inout("x0") x,in("x1") 7u64,in("x2") value as u64,in("x3") 1u64,options(nostack))}
 else {asm!("svc #0x140",inout("x0") x,in("x1") 7u64,in("x2") value as u64,in("x3") 1u64,options(nostack))}
 x
}
unsafe fn complete(write:bool,name:*mut u8,value:*mut u8)->u64 {
 loop {let x=io(write,name,value);if x!=(-11i64) as u64{return x}}
}
unsafe fn zero_page(p:*mut u64) {
 for i in 0..512 {if read_volatile(p.add(i))!=0 {stop(42)}}
}
unsafe fn setup(p:*mut u8,first:u8,marker:u64) {
 write_volatile(p.cast::<u64>(),marker);write_volatile(p.add(4088).cast::<u64>(),!marker);
 let name=p.add(4000);for (i,b) in [first,b'-',b'h',b'g',b'a',b't',b'e'].iter().enumerate() {write_volatile(name.add(i),*b)}
}
unsafe fn check(p:*mut u8,marker:u64) {
 if read_volatile(p.cast::<u64>())!=marker || read_volatile(p.add(4088).cast::<u64>())!=!marker {stop(42)}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(42)};
  if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(42)}
  if alloc()!=HEAP || alloc()!=(-17i64) as u64 || free(HEAP+8)!=(-22i64) as u64 {stop(42)}
  let p=HEAP as *mut u8;let marker=actor.wrapping_mul(0x100010001);zero_page(p.cast());setup(p,first,marker);
  let name=p.add(4000);let value=p.add(4016);
  // When the request FIFO is full -11 may mean no request was submitted.
  // Retry after a successful free; only -16 establishes an outstanding I/O.
  loop {
   if io(false,name,value)!=(-11i64) as u64 {stop(42)}
   let result=free(HEAP);
   if result==(-16i64) as u64 {break}
   if result!=0 || alloc()!=HEAP {stop(42)}
   zero_page(p.cast());setup(p,first,marker);
  }
  if complete(false,name,value)!=1 {stop(42)}
  write_volatile(value,b'r');if complete(true,name,value)!=1 {stop(42)}
  loop {if complete(false,name,value)!=1 {stop(42)}check(p,marker);match read_volatile(value){b'r'=>{},b'f'=>break,_=>stop(42)}}
  if free(HEAP)!=0 || free(HEAP)!=(-2i64) as u64 || alloc()!=HEAP {stop(42)}
  zero_page(p.cast());setup(p,first,marker);write_volatile(value,b'd');
  if complete(true,name,value)!=1 {stop(42)}
  loop {if complete(false,name,value)!=1 {stop(42)}check(p,marker);match read_volatile(value){b'd'=>{},b'q'=>break,_=>stop(42)}}
  if free(HEAP)!=0 {stop(42)}
  // Exercise Rust's actual alloc::Vec through the small fixture allocator.
  let layout=Layout::array::<u64>(16).unwrap_or_else(|_|stop(42));
  let memory=GlobalAlloc::alloc(&ALLOCATOR,layout).cast::<u64>();
  if memory.is_null() {stop(42)}
  let mut v=Vec::<u64>::from_raw_parts(memory,0,16);
  for i in 0..16 {v.as_mut_ptr().add(i).write(marker.wrapping_add(i as u64));}v.set_len(16);
  for i in 0..16 {if v.as_ptr().add(i).read()!=marker.wrapping_add(i as u64) {stop(42)}}
  drop(v);
  let mut page=match guest_api::HeapPage::allocate() {Ok(page)=>page,Err(_)=>stop(42)};
  zero_page(p.cast());setup(p,first,marker);
  let bytes=page.as_mut_slice();
  let (front,tail)=bytes.split_at_mut(4016);
  let name=core::str::from_utf8_unchecked(&front[4000..4007]);let output=&mut tail[..1];
  if guest_api::read_file(name,output)!=Ok(1) || output[0]!=b'q' {stop(42)}
  if guest_api::write_file(name,output)!=Ok(1) {stop(42)}
  if page.release().is_err() {stop(42)}
  if alloc()!=HEAP {stop(42)}zero_page(p.cast());if free(HEAP)!=0 {stop(42)}stop(0)
 }
}
