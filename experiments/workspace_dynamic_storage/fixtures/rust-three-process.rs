//! Three independently mapped Rust tasks, with isolated per-slot VFS access.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn once(write:bool,name:*mut u8,value:*mut u8)->u64 {
 let mut r=name as u64;
 if write {asm!("svc #0x141",inout("x0") r,in("x1") 6u64,in("x2") value as u64,in("x3") 1u64,options(nostack))}
 else {asm!("svc #0x140",inout("x0") r,in("x1") 6u64,in("x2") value as u64,in("x3") 1u64,options(nostack))}
 r
}
unsafe fn call(write:bool,name:*mut u8,value:*mut u8)->u64 {
 loop {let r=once(write,name,value);if r!=(-11i64) as u64{return r}}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(42)};
  let init=addr_of_mut!(INITIAL);let zero=addr_of_mut!(ZERO).cast::<u64>();
  if read_volatile(init)!=0x1122_3344_5566_7788 || (0..4).any(|i|read_volatile(zero.add(i))!=0){stop(42)}
  let mut name=[first,b'-',b'g',b'a',b't',b'e'];let mut value=[b'0'];
  let mut guard=[0xa5u8,0,0x5a];let local=guard.as_mut_ptr().add(1);
  if (local as usize)<0x50003000 || (local as usize)>=0x50004000{stop(42)}
  write_volatile(init,actor);write_volatile(zero,actor^0xff);write_volatile(local,b'0');
  if call(true,name.as_mut_ptr(),value.as_mut_ptr())!=1{stop(42)}
  // Other slots' names cannot be read or overwritten.
  for other in [b'a',b'b',b'c'] {
   if other==first {continue}name[0]=other;
   if call(false,name.as_mut_ptr(),value.as_mut_ptr())!=(-13i64) as u64{stop(42)}
   value[0]=b'X';if call(true,name.as_mut_ptr(),value.as_mut_ptr())!=(-13i64) as u64{stop(42)}
  }
  name[0]=first;
  loop {if call(false,name.as_mut_ptr(),value.as_mut_ptr())!=1{stop(42)}match value[0]{b'0'=>{},b'1'=>break,_=>stop(42)}}
  if read_volatile(init)!=actor || read_volatile(zero)!=actor^0xff || read_volatile(local)!=b'0' || read_volatile(guard.as_ptr())!=0xa5 || read_volatile(guard.as_ptr().add(2))!=0x5a {stop(42)}
  stop(0)
 }
}
