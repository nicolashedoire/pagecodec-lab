//! Two isolated tasks access explicitly bound document copies.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_unaligned,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;32]=[0;32];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
#[inline(never)] unsafe fn file(write:bool,name:*mut u8,value:*mut u8,capacity:u64)->u64 {
 loop {let mut r=name as u64;if write {asm!("svc #0x141",inout("x0") r,in("x1") 7u64,in("x2") value as u64,in("x3") capacity,options(nostack))}else{asm!("svc #0x140",inout("x0") r,in("x1") 7u64,in("x2") value as u64,in("x3") capacity,options(nostack))}if r!=(-11i64) as u64{return r}}
}
#[inline(never)] unsafe fn phase(name:*mut u8,value:*mut u8,byte:u8) {write_volatile(value,byte);if file(true,name,value,1)!=1{stop(42)}}
#[inline(never)] unsafe fn binding(name:*mut u8,data:*mut u8,stage:u8)->u64 {
 loop {let n=file(false,name,data,32);if n==1{continue}if n!=9{stop(42)}let id=u64::from_le(read_unaligned(data.cast::<u64>()));if id==0{stop(42)}let got=read_volatile(data.add(8));if got==stage||(stage==0&&(got==1||got==2)){return id}}
}
#[inline(never)] unsafe fn doc(write:bool,key:*mut u8,klen:u64,value:*mut u8,len:u64,id:u64)->u64 {
 loop {let mut r=key as u64;if write {asm!("svc #0x191",inout("x0") r,in("x1") klen,in("x2") value as u64,in("x3") len,in("x4") id,options(nostack))}else{asm!("svc #0x190",inout("x0") r,in("x1") klen,in("x2") value as u64,in("x3") len,in("x4") id,options(nostack))}if r!=(-11i64) as u64{return r}}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(42)};
  if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788{stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();if (0..32).any(|i|read_volatile(data.add(i))!=0){stop(42)}
  let mut name=[first,b'-',b'd',b'g',b'a',b't',b'e'];let mut title=*b"title";let mut key=if first==b'a'{*b"hproof"}else{*b"aproof"};
  let mut stack=[0xa5u8,0,0,0x5a];let value=stack.as_mut_ptr().add(1);
  if (value as usize)<0x50003000 || (value as usize)+2>0x50004000{stop(42)}
  if doc(false,title.as_mut_ptr(),5,data,32,0)!=(-13i64) as u64{stop(42)}
  phase(name.as_mut_ptr(),value,b'd');
  let first_id=binding(name.as_mut_ptr(),data,0);
  if first==b'a' {
   // Stage 2 deliberately conveys an old ID to a readmitted incarnation.
   // At stage 1 this function returns the live binding instead.
   let stage=read_volatile(data.add(8));
   if stage==2 {if doc(false,title.as_mut_ptr(),5,data,32,first_id)!=(-116i64) as u64{stop(42)}phase(name.as_mut_ptr(),value,b's');let fresh=binding(name.as_mut_ptr(),data,3);if fresh==first_id{stop(42)}
    write_volatile(value,b'H');write_volatile(value.add(1),b'2');if doc(true,key.as_mut_ptr(),6,value,2,fresh)!=2{stop(42)}
    if doc(false,key.as_mut_ptr(),6,data,32,fresh)!=2||read_volatile(data)!=b'H'||read_volatile(data.add(1))!=b'2'{stop(42)}
    if read_volatile(stack.as_ptr())!=0xa5||read_volatile(stack.as_ptr().add(3))!=0x5a{stop(42)}stop(0)
   }
  }
  if doc(false,title.as_mut_ptr(),5,data,32,first_id)!=4 || read_volatile(data)!=b'b' || read_volatile(data.add(1))!=b'a' || read_volatile(data.add(2))!=b's' || read_volatile(data.add(3))!=b'e'{stop(42)}
  let expected=if first==b'a'{b'H'}else{b'A'};write_volatile(value,expected);
  if doc(true,key.as_mut_ptr(),6,value,1,first_id)!=1{stop(42)}
  loop {if doc(false,key.as_mut_ptr(),6,data,32,first_id)!=1||read_volatile(data)!=expected{stop(42)}}
 }
}
