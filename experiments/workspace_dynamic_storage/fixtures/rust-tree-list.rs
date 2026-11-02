//! EL0 directory enumeration of immediate children at one tree generation.
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
unsafe fn once(path:*mut u8,path_len:u64,index:u64,output:*mut u8,capacity:u64,generation:u64,name_offset:u64)->u64 {
 let mut result=path as u64;
 asm!("svc #0x14c",inout("x0") result,in("x1") path_len,in("x2") index,in("x3") output as u64,in("x4") capacity,in("x5") generation,in("x6") name_offset,options(nostack));result
}
unsafe fn list(path:*mut u8,path_len:u64,index:u64,output:*mut u8,capacity:u64,generation:u64,name_offset:u64)->u64 {
 loop {let result=once(path,path_len,index,output,capacity,generation,name_offset);if result!=(-11i64) as u64{return result}}
}
unsafe fn generation()->u64 {let mut s=0u64;let g:u64;asm!("svc #0x145",inout("x0") s,lateout("x1") g,options(nostack));if s!=0 || g==0 {stop(2)}g}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(3)}
 let mut gate=[first,b'-',b'l',b'g',b'a',b't',b'e'];let mut phase=[first,b'-',b'l',b'p',b'h',b'a',b's',b'e'];
 let mut path=[b'/',first,b'/',b'd'];let mut other=path;other[1]=if first==b'a' {b'b'}else{b'a'};
 let mut missing=[b'/',first,b'/',b'm',b'i',b's',b's'];let mut invalid=[b'/',first,b'/',b'.',b'.',b'/',b'x'];
 let mut root=[b'/',first];let mut file_path=[b'/',first,b'/',b'd',b'/',b'a',b'a'];let mut out=[0xeeu8;32];let mut state=[0u8;1];let g=generation();
 if list(other.as_mut_ptr(),4,0,out.as_mut_ptr(),32,g,0)!=(-13i64) as u64 {stop(4)}
 if list(invalid.as_mut_ptr(),7,0,out.as_mut_ptr(),32,g,0)!=(-22i64) as u64 {stop(5)}
 if list(missing.as_mut_ptr(),7,0,out.as_mut_ptr(),32,g,0)!=(-2i64) as u64 {stop(6)}
 if list(file_path.as_mut_ptr(),7,0,out.as_mut_ptr(),32,g,0)!=(-20i64) as u64 {stop(7)}
 if list(path.as_mut_ptr(),4,0,out.as_mut_ptr(),32,g+1,0)!=(-116i64) as u64 {stop(8)}
 if list(path.as_mut_ptr(),4,0,out.as_mut_ptr(),0,g,0)!=(-22i64) as u64 {stop(9)}
 if list(path.as_mut_ptr(),4,0,0x50000fffu64 as *mut u8,2,g,0)!=(-14i64) as u64 {stop(10)}
 if list(path.as_mut_ptr(),4,0,out.as_mut_ptr(),1,g,0)!=(-22i64) as u64 || out.iter().any(|v|*v!=0xee) {stop(11)}
 if list(path.as_mut_ptr(),4,0,out.as_mut_ptr(),2,g,0)!=2 || out[..2]!=[0x81,b'a'] {stop(27)}
 out.fill(0xee);if list(path.as_mut_ptr(),4,0,out.as_mut_ptr(),2,g,1)!=2 || out[..2]!=[1,b'a'] {stop(28)}out.fill(0xee);
 if list(root.as_mut_ptr(),2,0,out.as_mut_ptr(),32,g,0)!=2 || out[..2]!=[0,b'd'] {stop(25)}
 if list(root.as_mut_ptr(),2,1,out.as_mut_ptr(),32,g,0)!=0 {stop(26)}out.fill(0xee);
 state[0]=b'r';if file(true,phase.as_mut_ptr(),8,state.as_mut_ptr(),1)!=1 {stop(12)}
 loop {if file(false,gate.as_mut_ptr(),7,state.as_mut_ptr(),1)!=1 {stop(13)}if state[0]==b'1' {break}if state[0]!=b'0' {stop(14)}}
 if once(path.as_mut_ptr(),4,0,out.as_mut_ptr(),32,g,0)!=(-11i64) as u64 {stop(15)}
 if once(path.as_mut_ptr(),4,1,out.as_mut_ptr(),32,g,0)!=(-22i64) as u64 {stop(16)}
 if once(path.as_mut_ptr(),4,0,out.as_mut_ptr(),32,g,1)!=(-22i64) as u64 {stop(29)}
 if list(path.as_mut_ptr(),4,0,out.as_mut_ptr(),32,g,0)!=3 || out[..3]!=[1,b'a',b'a'] {stop(17)}
 for x in &out[3..] {if *x!=0xee {stop(18)}}
 out.fill(0xee);if list(path.as_mut_ptr(),4,1,out.as_mut_ptr(),32,g,0)!=7 || out[..7]!=[0,b'b',b'r',b'a',b'n',b'c',b'h'] {stop(19)}
 for x in &out[7..] {if *x!=0xee {stop(20)}}
 out.fill(0xee);if list(path.as_mut_ptr(),4,2,out.as_mut_ptr(),32,g,0)!=3 || out[..3]!=[1,b'z',b'z'] {stop(21)}
 out.fill(0xee);if list(path.as_mut_ptr(),4,3,out.as_mut_ptr(),32,g,0)!=32 || out[0]!=0x81 || out[1..].iter().any(|v|*v!=b'z') {stop(22)}
 out.fill(0xee);let path_string=core::str::from_utf8_unchecked(&path);
 if guest_api::tree_list_fragment(path_string,3,&mut out,g,31)!=Ok(2) || out[..2]!=[1,b'z'] || out[2..].iter().any(|v|*v!=0xee) {stop(30)}
 out.fill(0xee);if list(path.as_mut_ptr(),4,3,out.as_mut_ptr(),32,g,32)!=(-22i64) as u64 || out.iter().any(|v|*v!=0xee) {stop(31)}
 if list(path.as_mut_ptr(),4,4,out.as_mut_ptr(),32,g,1)!=(-22i64) as u64 {stop(32)}
 if list(path.as_mut_ptr(),4,4,out.as_mut_ptr(),32,g,0)!=0 || out.iter().any(|v|*v!=0xee) {stop(23)}
 state[0]=b'd';if file(true,phase.as_mut_ptr(),8,state.as_mut_ptr(),1)!=1 {stop(24)}stop(0)
}}
