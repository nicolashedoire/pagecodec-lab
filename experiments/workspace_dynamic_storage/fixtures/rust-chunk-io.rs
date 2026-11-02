//! Real EL0 file-offset requests; two VM boots share a disposable disk.
#![no_std]
#![no_main]
#[path="../guest_api.rs"] mod guest_api;
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn once(kind:u64,name:*mut u8,n:u64,data:*mut u8,len:u64,offset:u64)->u64 {
 let mut result=name as u64;
 match kind {
  0=>asm!("svc #0x140",inout("x0") result,in("x1") n,in("x2") data as u64,in("x3") len,options(nostack)),
  1=>asm!("svc #0x141",inout("x0") result,in("x1") n,in("x2") data as u64,in("x3") len,options(nostack)),
  2=>asm!("svc #0x142",inout("x0") result,in("x1") n,in("x2") data as u64,in("x3") len,in("x4") offset,options(nostack)),
  3=>asm!("svc #0x143",inout("x0") result,in("x1") n,in("x2") data as u64,in("x3") len,in("x4") offset,options(nostack)),
  _=>stop(1),
 }result
}
unsafe fn call(kind:u64,name:*mut u8,n:u64,data:*mut u8,len:u64,offset:u64)->u64 {
 loop {let r=once(kind,name,n,data,len,offset);if r!=(-11i64) as u64{return r}}
}
fn expected(index:usize)->u8 {if (30..40).contains(&index) {b"CHANGED123"[index-30]}else{(index as u8)^0x5a}}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 if actor!=0xa1a1 || read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(2)}
 let mut name=*b"a-chunk";let mut gate=*b"a-ogate";let mut missing=*b"a-missing";let empty=*b"a-empty";
 let mut state=[0u8];let mut data=[0u8;32];
 if call(0,gate.as_mut_ptr(),7,state.as_mut_ptr(),1,0)!=1 {stop(3)}
 match state[0] {
  b'0'=>{
   let empty_string=core::str::from_utf8_unchecked(&empty);
   if guest_api::write_file(empty_string,&[])!=Ok(0) {stop(27)}
   if call(3,missing.as_mut_ptr(),9,data.as_mut_ptr(),32,0)!=(-2i64) as u64 {stop(4)}
   if call(2,missing.as_mut_ptr(),9,data.as_mut_ptr(),1,1)!=(-22i64) as u64 {stop(5)}
   for chunk in 0..16usize {
    let offset=chunk*32;let count=32;
    for i in 0..count {data[i]=((offset+i) as u8)^0x5a}
    if call(2,name.as_mut_ptr(),7,data.as_mut_ptr(),count as u64,offset as u64)!=count as u64 {stop(6)}
    if chunk==0 && call(2,name.as_mut_ptr(),7,data.as_mut_ptr(),1,33)!=(-22i64) as u64 {stop(8)}
   }
   data[..10].copy_from_slice(b"CHANGED123");
   let name_string=core::str::from_utf8_unchecked(&name);
   if guest_api::write_at(name_string,&data[..10],30)!=Ok(10) {stop(7)}
   if guest_api::write_at(name_string,&[],0)!=Ok(0) {stop(26)}
   if call(2,name.as_mut_ptr(),7,data.as_mut_ptr(),1,512)!=(-27i64) as u64 {stop(9)}
   if call(2,name.as_mut_ptr(),7,data.as_mut_ptr(),1,u64::MAX)!=(-27i64) as u64 {stop(10)}
   // A nonzero range may not cross from private data into the code page.
   if call(2,name.as_mut_ptr(),7,0x50000fffu64 as *mut u8,2,0)!=(-14i64) as u64 {stop(24)}
   if call(3,name.as_mut_ptr(),7,0x50000fffu64 as *mut u8,2,0)!=(-14i64) as u64 {stop(25)}
   state[0]=b'w';if call(1,gate.as_mut_ptr(),7,state.as_mut_ptr(),1,0)!=1 {stop(11)}
   // The host stops this incarnation before rebooting the same test disk.
   loop {if call(0,gate.as_mut_ptr(),7,state.as_mut_ptr(),1,0)!=1 || state[0]!=b'w' {stop(12)}}
  },
  b'r'=>{
   let empty_string=core::str::from_utf8_unchecked(&empty);
   if guest_api::read_at(empty_string,&mut data,0)!=Ok(0) {stop(28)}
   // The first read must be genuinely submitted before checking poll identity:
   // this scenario uses one actor and waits for all preceding I/O to finish.
   if once(3,name.as_mut_ptr(),7,data.as_mut_ptr(),32,0)!=(-11i64) as u64 {stop(13)}
   if once(3,name.as_mut_ptr(),7,data.as_mut_ptr(),32,1)!=(-22i64) as u64 {stop(14)}
   if call(3,name.as_mut_ptr(),7,data.as_mut_ptr(),32,0)!=32 {stop(15)}
   for i in 0..32 {if data[i]!=expected(i) {stop(16)}}
   for chunk in 1..16usize {
    let offset=chunk*32;data.fill(0xee);
    if call(3,name.as_mut_ptr(),7,data.as_mut_ptr(),32,offset as u64)!=32 {stop(17)}
    for i in 0..32 {if data[i]!=expected(offset+i) {stop(18)}}
   }
   data.fill(0xee);
   let name_string=core::str::from_utf8_unchecked(&name);
   if guest_api::read_at(name_string,&mut data,500)!=Ok(12) {stop(19)}
   for i in 0..12 {if data[i]!=expected(500+i) {stop(19)}}
   for i in 12..32 {if data[i]!=0xee {stop(19)}}
   data.fill(0xee);
   if call(3,name.as_mut_ptr(),7,data.as_mut_ptr(),32,512)!=0 || data.iter().any(|x|*x!=0xee) {stop(20)}
   if call(3,name.as_mut_ptr(),7,data.as_mut_ptr(),1,u64::MAX)!=(-22i64) as u64 {stop(21)}
   state[0]=b'v';if call(1,gate.as_mut_ptr(),7,state.as_mut_ptr(),1,0)!=1 {stop(22)}stop(0)
  },
  _=>stop(23),
 }
}}
