//! Transactional staging from two isolated ELF processes into the tree.
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
unsafe fn begin(path:*mut u8,len:u64,size:u64,sum:u64,generation:u64)->u64 {
 loop {let mut r=path as u64;asm!("svc #0x146",inout("x0") r,in("x1") len,in("x2") size,in("x3") sum,in("x4") generation,options(nostack));if r!=(-11i64) as u64{return r}}
}
unsafe fn append(id:u64,offset:u64,data:*mut u8,len:u64)->u64 {
 loop {let mut r=offset;asm!("svc #0x147",inout("x0") r,in("x1") data as u64,in("x2") len,in("x3") id,options(nostack));if r!=(-11i64) as u64{return r}}
}
unsafe fn finish(commit:bool,id:u64)->u64 {
 loop {let mut r=id;
  if commit {asm!("svc #0x148",inout("x0") r,options(nostack))}else{asm!("svc #0x149",inout("x0") r,options(nostack))}
  if r!=(-11i64) as u64{return r}
 }
}
unsafe fn generation()->u64 {
 let mut status=0u64;let g:u64;asm!("svc #0x145",inout("x0") status,lateout("x1") g,options(nostack));if status!=0 || g==0 {stop(2)}g
}
fn value(first:u8,i:usize)->u8 {let v=if first==b'a' {i.wrapping_mul(37).wrapping_add(5)}else{i.wrapping_mul(23).wrapping_add(77)};(v^(i>>8)) as u8}
fn checksum(first:u8,size:usize)->u64 {let mut h=0xcbf29ce484222325u64;for i in 0..size {h=(h^u64::from(value(first,i))).wrapping_mul(0x100000001b3)}h}
unsafe fn stage(path:*mut u8,first:u8,size:usize,g:u64,data:&mut [u8;32])->u64 {
 let id=begin(path,8,size as u64,checksum(first,size),g);if id==0 || (id as i64)<0 {stop(3)}
 for offset in (0..size).step_by(32) {
  let n=core::cmp::min(size-offset,32);for i in 0..n {data[i]=value(first,offset+i)}
  if append(id,offset as u64,data.as_mut_ptr(),n as u64)!=n as u64 {stop(4)}
 }
 id
}
unsafe fn phase(name:&mut [u8;8],state:&mut [u8;1],v:u8) {state[0]=v;if file(true,name.as_mut_ptr(),8,state.as_mut_ptr(),1)!=1 {stop(5)}}
unsafe fn await_gate(name:&mut [u8;7],state:&mut [u8;1],ready:u8,wanted:u8) {
 loop {if file(false,name.as_mut_ptr(),7,state.as_mut_ptr(),1)!=1 {stop(6)}if state[0]==wanted {return}if state[0]!=ready {stop(7)}}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(8)}
 let mut gate=[first,b'-',b'u',b'g',b'a',b't',b'e'];let mut status=[first,b'-',b'u',b'p',b'h',b'a',b's',b'e'];
 let mut state=[0u8;1];let mut data=[0u8;32];let mut path=[b'/',first,b'/',b'w',b'r',b'i',b't',b'e'];
 let mut other=path;other[1]=if first==b'a' {b'b'}else{b'a'};
 if file(false,gate.as_mut_ptr(),7,state.as_mut_ptr(),1)!=1 {stop(9)}
 let mode=state[0];let g=generation();
 if first==b'a' && mode==b'p' {
  // An interrupted older incarnation must have surrendered its stage.
  let id=stage(path.as_mut_ptr(),first,32,g,&mut data);if finish(false,id)!=0 {stop(10)}phase(&mut status,&mut state,b'p');stop(0)
 }
 if mode!=b'0' && mode!=b'L' {stop(11)}
 if begin(other.as_mut_ptr(),8,1,checksum(first,1),g)!=(-13i64) as u64 {stop(12)}
 if begin(path.as_mut_ptr(),8,16385,checksum(first,1),g)!=(-27i64) as u64 {stop(13)}
 if begin(path.as_mut_ptr(),8,1,checksum(first,1),g+1)!=(-116i64) as u64 {stop(14)}
 // Aborted stages must not be published, and a second stage must succeed.
 let mut aborted=[b'/',first,b'/',b'a',b'b',b'o',b'r',b't'];
 let old=stage(aborted.as_mut_ptr(),first,32,g,&mut data);if finish(false,old)!=0 {stop(15)}
 let size=if first==b'a' && mode==b'L' {16384}else if first==b'b' {513}else{512};
 let id=begin(path.as_mut_ptr(),8,size as u64,checksum(first,size),g);if id==0 || (id as i64)<0 || id==old {stop(16)}
 if append(old,0,data.as_mut_ptr(),1)!=(-116i64) as u64 || finish(true,old)!=(-116i64) as u64 || finish(false,old)!=(-116i64) as u64 {stop(28)}
 if begin(path.as_mut_ptr(),8,size as u64,checksum(first,size),g)!=(-16i64) as u64 {stop(17)}
 if append(id,1,data.as_mut_ptr(),1)!=(-22i64) as u64 {stop(18)}
 if append(id,0,0x50000fffu64 as *mut u8,2)!=(-14i64) as u64 {stop(19)}
 if append(id,0,data.as_mut_ptr(),0)!=(-22i64) as u64 {stop(20)}
 for offset in (0..size).step_by(32) {let n=core::cmp::min(size-offset,32);for i in 0..n {data[i]=value(first,offset+i)}if append(id,offset as u64,data.as_mut_ptr(),n as u64)!=n as u64 {stop(21)}}
 phase(&mut status,&mut state,b'r');
 if first==b'a' {
  await_gate(&mut gate,&mut state,mode,b'a');if finish(true,id)!=0 || generation()!=g+1 {stop(22)}phase(&mut status,&mut state,b'd');
  await_gate(&mut gate,&mut state,b'a',b'x');let next=generation();
  let mut scratch=[b'/',first,b'/',b's',b'c',b'r',b'a',b't'];
  let _scratch_id=stage(scratch.as_mut_ptr(),first,32,next,&mut data);phase(&mut status,&mut state,b's');
  // The controller stops this incarnation while a stage is owned.
  loop {if file(false,gate.as_mut_ptr(),7,state.as_mut_ptr(),1)!=1 || state[0]!=b'x' {stop(23)}}
 } else {
  await_gate(&mut gate,&mut state,mode,b'b');if finish(true,id)!=(-116i64) as u64 {stop(24)}
  if finish(false,id)!=0 {stop(25)}let next=generation();if next!=g+1 {stop(26)}
  // Retry through the reusable Rust upload wrapper, while retaining the raw
  // SVC exercises above for stale-ID and failed-commit behavior.
  let path_string=core::str::from_utf8_unchecked(&path);
  let mut upload=match guest_api::TreeUpload::begin(path_string,size,checksum(first,size),next) {Ok(upload)=>upload,Err(_)=>stop(27)};
  for offset in (0..size).step_by(32) {
   let n=core::cmp::min(size-offset,32);for i in 0..n {data[i]=value(first,offset+i)}
   if upload.append(&data[..n])!=Ok(n) {stop(27)}
  }
  if upload.commit()!=Ok(()) || generation()!=next+1 {stop(27)}
  phase(&mut status,&mut state,b'e');await_gate(&mut gate,&mut state,b'b',b'q');stop(0)
 }
}}
