//! Actual `alloc` crate objects in the guest. Compile only; never run on host.
#![no_std]
#![no_main]
extern crate alloc;
#[path="../guest_api.rs"] mod guest_api;
#[path="../guest_allocator.rs"] mod allocator;
use alloc::{boxed::Box,vec::Vec,alloc::{alloc,dealloc}};
use core::{alloc::{GlobalAlloc,Layout},arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[global_allocator] static ALLOC:allocator::GuestAllocator=allocator::GuestAllocator::new();
#[used] #[link_section=".data.first"] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
#[inline(never)] unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
#[inline(never)] fn observe(ptr:*const u8) {unsafe {asm!("// guest pointer {p}",p=in(reg) ptr,options(nostack,readonly))}}
#[inline(never)] fn opaque(mut value:usize)->usize {unsafe {asm!("// guest value {v}",v=inout(reg) value,options(nostack))};value}
#[no_mangle] pub extern "C" fn _start(_:u64,actor:u64)->! {unsafe {
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 {stop(1)}
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(2)};
 // Page zero is held by an independent safe owner. Allocator objects must
 // acquire a different page without borrowing or overwriting this one.
 let mut external=match guest_api::HeapPage::allocate_index(0) {Ok(p)=>p,Err(_)=>stop(3)};
 write_volatile(external.as_mut_slice().as_mut_ptr(),0x5a);
 let a=Box::new(0x1122_3344_5566_7788u64);
 let b=Box::new(0x8899_aabb_ccdd_eeffu64);
 let ap=Box::into_raw(a);let bp=Box::into_raw(b);
 observe(ap.cast());observe(bp.cast());
 if ap==bp || read_volatile(ap)!=0x1122_3344_5566_7788 || read_volatile(bp)!=0x8899_aabb_ccdd_eeff {stop(4)}
 let mut v:Vec<u8>=Vec::new();
 for i in 0..33u8 {v.push(i ^ 0xa5)}
 observe(v.as_ptr());
 if (0..33).any(|i|read_volatile(v.as_ptr().add(i))!=(i as u8 ^ 0xa5)) {stop(5)}
 // The first gap must be reusable while b and v remain alive.
 drop(Box::from_raw(ap));
 let c=Box::new(0x0102_0304_0506_0708u64);let cp=Box::into_raw(c);observe(cp.cast());
 if cp!=ap || read_volatile(cp)!=0x0102_0304_0506_0708 || read_volatile(bp)!=0x8899_aabb_ccdd_eeff {stop(6)}
 let layout=Layout::from_size_align_unchecked(73,256);let aligned=alloc(layout);observe(aligned);
 if aligned.is_null() || (aligned as usize)&255!=0 {stop(7)}
 // Object extents must be disjoint and stay on the allocator-owned page,
 // while page 0 remains held by the independent HeapPage owner.
 let ranges=[(cp as usize,8usize),(bp as usize,8usize),(v.as_ptr() as usize,v.capacity()),(aligned as usize,73usize)];
 for i in 0..4 {let (start,len)=ranges[i];let end=match start.checked_add(len) {Some(end)=>end,None=>stop(17)};
  if start<0x50005000 || end>0x50006000 {stop(17)}
  for j in 0..i {let (other,other_len)=ranges[j];if start<other+other_len && other<end {stop(17)}}
 }
 write_volatile(aligned,0x74);write_volatile(aligned.add(72),0x39);
 if read_volatile(aligned)!=0x74 || read_volatile(aligned.add(72))!=0x39 {stop(8)}
 if (0..33).any(|i|read_volatile(v.as_ptr().add(i))!=(i as u8 ^ 0xa5)) {stop(5)}
 let too_large=Layout::from_size_align_unchecked(opaque(4097),8);
 // alloc::alloc::alloc may be optimized under compiler allocation assumptions.
 // Call this allocator's trait implementation directly to observe refusal.
 let oversized=GlobalAlloc::alloc(&ALLOC,too_large);observe(oversized);
 if !oversized.is_null() {stop(9)}
 // Use a single per-actor gate: the controller reads the exact Vec bytes,
 // acknowledges with g, then sees r while every allocation remains alive.
 let mut gate=[first,b'-',b'r',b'g',b'a',b't',b'e'];
 let gate_name=core::str::from_utf8_unchecked_mut(&mut gate);
 if guest_api::write_file(gate_name,&v[..16])!=Ok(16) {stop(10)}
 if read_volatile(external.as_mut_slice().as_mut_ptr())!=0x5a {stop(11)}
 let mut pending=[0u8;16];
 loop {
  match guest_api::read_file(gate_name,&mut pending) {
   Ok(16) if (0..16).all(|i|pending[i]==(i as u8 ^ 0xa5))=>core::hint::spin_loop(),
   Ok(1) if pending[0]==b'g'=>break,
   _=>stop(16),
  }
 }
 let mut phase=[b'r'];
 if guest_api::write_file(gate_name,&phase)!=Ok(1) {stop(13)}
 loop {
  match guest_api::read_file(gate_name,&mut phase) {
   Ok(1) if phase[0]==b'r'=>core::hint::spin_loop(),
   Ok(1) if phase[0]==b'q'=>break,
   _=>stop(14),
  }
 }
 if read_volatile(cp)!=0x0102_0304_0506_0708 || read_volatile(bp)!=0x8899_aabb_ccdd_eeff || read_volatile(external.as_mut_slice().as_mut_ptr())!=0x5a {stop(15)}
 dealloc(aligned,layout);drop(Box::from_raw(cp));drop(Box::from_raw(bp));drop(v);
 if external.release().is_err() {stop(12)}
 stop(0)
}}
