//! Two independent processes create/remove names in their own tree scope.
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
unsafe fn operation(create:bool,path:*mut u8,len:u64,generation:u64)->u64 {
 loop {let mut result=path as u64;
  if create {asm!("svc #0x14a",inout("x0") result,in("x1") len,in("x2") generation,options(nostack))}
  else {asm!("svc #0x14b",inout("x0") result,in("x1") len,in("x2") generation,options(nostack))}
  if result!=(-11i64) as u64{return result}
 }
}
unsafe fn generation()->u64 {let mut s=0u64;let g:u64;asm!("svc #0x145",inout("x0") s,lateout("x1") g,options(nostack));if s!=0 || g==0 {stop(2)}g}
unsafe fn advanced(old:u64)->u64 {let next=generation();if next!=old+1 {stop(3)}next}
unsafe fn signal(name:&mut [u8;8],state:&mut [u8;1],v:u8) {state[0]=v;if file(true,name.as_mut_ptr(),8,state.as_mut_ptr(),1)!=1 {stop(4)}}
unsafe fn wait(name:&mut [u8;7],state:&mut [u8;1],ready:u8,wanted:u8) {
 loop {if file(false,name.as_mut_ptr(),7,state.as_mut_ptr(),1)!=1 {stop(5)}if state[0]==wanted {return}if state[0]!=ready {stop(6)}}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(7)}
 let mut gate=[first,b'-',b'm',b'g',b'a',b't',b'e'];let mut phase=[first,b'-',b'm',b'p',b'h',b'a',b's',b'e'];let mut state=[0u8];
 let mut base=[b'/',first,b'/',b't',b'e',b's',b't'];let mut child=[b'/',first,b'/',b't',b'e',b's',b't',b'/',b's',b'u',b'b'];
 let mut keep=[b'/',first,b'/',b'k',b'e',b'e',b'p'];let mut erase=[b'/',first,b'/',b'e',b'r',b'a',b's',b'e'];
 let mut missing=[b'/',first,b'/',b'm',b'i',b's',b's'];let mut invalid=[b'/',first,b'/',b'.',b'.',b'/',b'x'];
 let mut other=base;other[1]=if first==b'a' {b'b'}else{b'a'};
 let initial=generation();
 if operation(true,other.as_mut_ptr(),7,initial)!=(-13i64) as u64 || operation(false,other.as_mut_ptr(),7,initial)!=(-13i64) as u64 {stop(8)}
 if operation(true,invalid.as_mut_ptr(),7,initial)!=(-22i64) as u64 {stop(9)}
 if operation(false,missing.as_mut_ptr(),7,initial)!=(-2i64) as u64 {stop(10)}
 if operation(true,base.as_mut_ptr(),7,initial+1)!=(-116i64) as u64 {stop(11)}
 signal(&mut phase,&mut state,b'r');let go=if first==b'a' {b'a'}else{b'b'};wait(&mut gate,&mut state,b'0',go);
 if first==b'b' && operation(true,base.as_mut_ptr(),7,initial)!=(-116i64) as u64 {stop(12)}
 let mut g=generation();
 if operation(true,base.as_mut_ptr(),7,g)!=0 {stop(13)}g=advanced(g);
 if operation(true,base.as_mut_ptr(),7,g)!=(-17i64) as u64 {stop(14)}
 if operation(true,child.as_mut_ptr(),11,g)!=0 {stop(15)}g=advanced(g);
 if operation(false,base.as_mut_ptr(),7,g)!=(-39i64) as u64 {stop(16)}
 if operation(false,child.as_mut_ptr(),11,g-1)!=(-116i64) as u64 {stop(17)}
 if operation(false,child.as_mut_ptr(),11,g)!=0 {stop(18)}g=advanced(g);
 if operation(false,base.as_mut_ptr(),7,g)!=0 {stop(19)}g=advanced(g);
 let erase_path=core::str::from_utf8_unchecked(&erase);
 if guest_api::tree_remove(erase_path,g)!=Ok(()) {stop(20)}g=advanced(g);
 let keep_path=core::str::from_utf8_unchecked(&keep);
 if guest_api::tree_mkdir(keep_path,g)!=Ok(()) {stop(21)}let _=advanced(g);
 signal(&mut phase,&mut state,b'd');wait(&mut gate,&mut state,go,b'q');stop(0)
}}
