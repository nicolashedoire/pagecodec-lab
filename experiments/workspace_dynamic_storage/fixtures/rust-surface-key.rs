//! Independent surface clients test virtual keyboard isolation and lifecycle.
#![no_std]
#![no_main]
#[allow(dead_code)] #[path="../guest_api.rs"] mod guest_api;
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn file(write:bool,name:*mut u8,len:u64,data:*mut u8,n:u64)->u64 {
 loop {let mut r=name as u64;
  if write {asm!("svc #0x141",inout("x0") r,in("x1") len,in("x2") data as u64,in("x3") n,options(nostack))}
  else {asm!("svc #0x140",inout("x0") r,in("x1") len,in("x2") data as u64,in("x3") n,options(nostack))}
  if r!=(-11i64) as u64{return r}
 }
}
unsafe fn simple(svc:u8,id:u64)->u64 {loop {let mut r=id;if svc==0 {asm!("svc #0x1a1",inout("x0") r,options(nostack))}else {asm!("svc #0x1a3",inout("x0") r,options(nostack))}if r!=(-11i64) as u64{return r}}}
unsafe fn key(id:u64,out:*mut u8)->u64 {loop {let mut r=id;asm!("svc #0x1a5",inout("x0") r,in("x1") out as u64,options(nostack));if r!=(-11i64) as u64{return r}}}
unsafe fn signal(name:&mut [u8;8],flag:u8) {let mut x=[flag];if file(true,name.as_mut_ptr(),8,x.as_mut_ptr(),1)!=1 {stop(29)}}
unsafe fn wait(name:&mut [u8;7],input:&mut [u8;9],flag:u8,previous:u8) {loop {if file(false,name.as_mut_ptr(),7,input.as_mut_ptr(),9)!=9 {stop(28)}if input[8]==flag {break}if input[8]!=previous {stop(27)}}}
fn word(a:&[u8],i:usize)->u32 {u32::from_le_bytes([a[i],a[i+1],a[i+2],a[i+3]])}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(2)}
 let mut gate=[first,b'-',b'k',b'g',b'a',b't',b'e'];let mut phase=[first,b'-',b'k',b'p',b'h',b'a',b's',b'e'];let mut input=[0u8;9];let mut ev=[0u8;12];
 let surface=match guest_api::Surface::create(128,64){Ok(s)=>s,Err(_)=>stop(3)};let id=surface.id();if surface.present().is_err(){stop(3)}
 let mut bytes=id.to_le_bytes();if file(true,phase.as_mut_ptr(),8,bytes.as_mut_ptr(),8)!=8 {stop(4)}
 loop {if file(false,gate.as_mut_ptr(),7,input.as_mut_ptr(),9)!=9 {stop(5)}if input[8]==b'1'||input[8]==b'p' {break}if input[8]!=b'0' {stop(6)}}
 let peer=u64::from_le_bytes([input[0],input[1],input[2],input[3],input[4],input[5],input[6],input[7]]);
 if peer==0 || peer==id {stop(7)}
 if input[8]==b'p' {if key(peer,ev.as_mut_ptr())!=(-116i64) as u64 || key(id,ev.as_mut_ptr())!=0 || simple(0,id)!=0 {stop(8)}signal(&mut phase,b'p');stop(0)}
 if key(peer,ev.as_mut_ptr())!=(-13i64) as u64 {stop(9)}
 signal(&mut phase,b'r');wait(&mut gate,&mut input,b'n',b'1');
 if surface.poll_key()!=Ok(None) {stop(10)}signal(&mut phase,b'n');
 wait(&mut gate,&mut input,b'k',b'n');
 if first==b'a' {for value in [1u32,0u32] {if key(id,ev.as_mut_ptr())!=12 || word(&ev,0)!=30 || word(&ev,4)!=value || word(&ev,8)!=0 {stop(11)}}}
 if key(id,ev.as_mut_ptr())!=0 {stop(12)}signal(&mut phase,b'k');
 wait(&mut gate,&mut input,b't',b'k');
 if first==b'a' {for value in [1u32,0u32] {if key(id,ev.as_mut_ptr())!=12 || word(&ev,0)!=30 || word(&ev,4)!=value || word(&ev,8)!=0 {stop(18)}}}
 if key(id,ev.as_mut_ptr())!=0 {stop(19)}signal(&mut phase,b't');
 wait(&mut gate,&mut input,b'u',b't');if key(id,ev.as_mut_ptr())!=0 {stop(17)}signal(&mut phase,b'u');
 wait(&mut gate,&mut input,b'f',b'u');
 if first==b'a' {
  // Overflow resets key state, followed by a fresh press/release sequence.
  let reset=guest_api::KeyEvent{code:guest_api::KEY_RESET,value:0,source:0};
  if surface.poll_key()!=Ok(Some(reset)) {stop(13)}
  for value in [1u32,0u32] {if key(id,ev.as_mut_ptr())!=12 || word(&ev,0)!=30 || word(&ev,4)!=value || word(&ev,8)!=0 {stop(14)}}
 }
 if key(id,ev.as_mut_ptr())!=0 {stop(15)}signal(&mut phase,b'f');
 wait(&mut gate,&mut input,b'q',b'f');if simple(0,id)!=0 {stop(16)}stop(0)
}}
