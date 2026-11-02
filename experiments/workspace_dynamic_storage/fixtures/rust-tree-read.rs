//! Independent guest processes read scoped hierarchical files via SVC 0x144.
#![no_std]
#![no_main]
#[path="../guest_api.rs"] mod guest_api;
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn file(write:bool,name:*mut u8,len:u64,data:*mut u8,n:u64)->u64 {
 loop {let mut result=name as u64;
  if write {asm!("svc #0x141",inout("x0") result,in("x1") len,in("x2") data as u64,in("x3") n,options(nostack))}
  else {asm!("svc #0x140",inout("x0") result,in("x1") len,in("x2") data as u64,in("x3") n,options(nostack))}
  if result!=(-11i64) as u64{return result}
 }
}
unsafe fn once(path:*mut u8,path_len:u64,output:*mut u8,capacity:u64,offset:u64,generation:u64)->u64 {
 let mut result=path as u64;
 asm!("svc #0x144",inout("x0") result,in("x1") path_len,in("x2") output as u64,in("x3") capacity,in("x4") offset,in("x5") generation,options(nostack));result
}
unsafe fn read(path:*mut u8,path_len:u64,output:*mut u8,capacity:u64,offset:u64,generation:u64)->u64 {
 loop {let result=once(path,path_len,output,capacity,offset,generation);if result!=(-11i64) as u64{return result}}
}
fn expected(index:usize,first:u8)->u8 {
 let value=if first==b'a' {index.wrapping_mul(31).wrapping_add(3)}else{index.wrapping_mul(19).wrapping_add(79)};
 (value^(index>>8)) as u8
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(2)}
 let mut gate=[first,b'-',b't',b'g',b'a',b't',b'e'];let mut phase=[first,b'-',b't',b'p',b'h',b'a',b's',b'e'];
 let mut input=[0u8;1];let mut output=[0u8;32];
 if file(false,gate.as_mut_ptr(),7,input.as_mut_ptr(),1)!=1 {stop(3)}
 let generation=match guest_api::tree_generation() {Ok(Some(generation))=>generation,Ok(None)=>0,Err(_)=>stop(4)};
 let mut own=[b'/',first,b'/',b'd',b'a',b't',b'a'];
 let mut other=own;other[1]=if first==b'a' {b'b'}else{b'a'};
 let mut absent=[b'/',first,b'/',b'm',b'i',b's',b's'];
 let mut directory=[b'/',first,b'/',b'd',b'i',b'r'];
 let mut invalid=[b'/',first,b'/',b'.',b'.',b'/',b'x'];
 if input[0]==b'c' {
  if generation!=0 || read(own.as_mut_ptr(),7,output.as_mut_ptr(),32,0,1)!=(-19i64) as u64 {stop(5)}
  input[0]=b'c';if file(true,phase.as_mut_ptr(),8,input.as_mut_ptr(),1)!=1 {stop(6)}stop(0)
 }
 if input[0]!=b'0' || generation==0 {stop(7)}
 if read(other.as_mut_ptr(),7,output.as_mut_ptr(),32,0,generation)!=(-13i64) as u64 {stop(8)}
 if read(invalid.as_mut_ptr(),7,output.as_mut_ptr(),32,0,generation)!=(-22i64) as u64 {stop(9)}
 if read(absent.as_mut_ptr(),7,output.as_mut_ptr(),32,0,generation)!=(-2i64) as u64 {stop(10)}
 if read(directory.as_mut_ptr(),6,output.as_mut_ptr(),32,0,generation)!=(-21i64) as u64 {stop(11)}
 if read(own.as_mut_ptr(),7,output.as_mut_ptr(),32,0,generation+1)!=(-116i64) as u64 {stop(12)}
 if read(own.as_mut_ptr(),7,0x50000fffu64 as *mut u8,2,0,generation)!=(-14i64) as u64 {stop(13)}
 input[0]=b'r';if file(true,phase.as_mut_ptr(),8,input.as_mut_ptr(),1)!=1 {stop(14)}
 loop {
  if file(false,gate.as_mut_ptr(),7,input.as_mut_ptr(),1)!=1 {stop(15)}
  if input[0]==b'1' {break}if input[0]!=b'0' {stop(16)}
 }
 // Polling a different offset must not consume the original request.
 if once(own.as_mut_ptr(),7,output.as_mut_ptr(),32,0,generation)!=(-11i64) as u64 {stop(17)}
 if once(own.as_mut_ptr(),7,output.as_mut_ptr(),32,1,generation)!=(-22i64) as u64 {stop(18)}
 if read(own.as_mut_ptr(),7,output.as_mut_ptr(),32,0,generation)!=32 {stop(19)}
 for i in 0..32 {if output[i]!=expected(i,first) {stop(20)}}
 for chunk in 1..17usize {
  let offset=chunk*32;let count=if chunk==16 {1}else{32};output.fill(0xee);
  let got=if chunk==16 {
   let path=core::str::from_utf8_unchecked(&own);
   match guest_api::tree_read_at(path,&mut output,offset,generation) {Ok(n)=>n as u64,Err(_)=>stop(21)}
  } else {read(own.as_mut_ptr(),7,output.as_mut_ptr(),32,offset as u64,generation)};
  if got!=count as u64 {stop(21)}
  for i in 0..count {if output[i]!=expected(offset+i,first) {stop(22)}}
  for i in count..32 {if output[i]!=0xee {stop(23)}}
 }
 output.fill(0xee);
 if read(own.as_mut_ptr(),7,output.as_mut_ptr(),32,513,generation)!=0 || output.iter().any(|x|*x!=0xee) {stop(24)}
 input[0]=b'd';if file(true,phase.as_mut_ptr(),8,input.as_mut_ptr(),1)!=1 {stop(25)}stop(0)
}}
