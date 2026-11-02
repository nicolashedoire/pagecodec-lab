//! Actual independent EL0 clients draw and receive pointer events in the VM.
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
unsafe fn fill(id:u64,x:u64,y:u64,w:u64,h:u64,color:u64)->u64 {loop {let mut r=id;asm!("svc #0x1a2",inout("x0") r,in("x1") x,in("x2") y,in("x3") w,in("x4") h,in("x5") color,options(nostack));if r!=(-11i64) as u64{return r}}}
unsafe fn present(id:u64)->u64 {loop {let mut r=id;asm!("svc #0x1a3",inout("x0") r,options(nostack));if r!=(-11i64) as u64{return r}}}
unsafe fn event(id:u64,out:*mut u8)->u64 {loop {let mut r=id;asm!("svc #0x1a4",inout("x0") r,in("x1") out as u64,options(nostack));if r!=(-11i64) as u64{return r}}}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(2)}
 let mut gate=[first,b'-',b's',b'g',b'a',b't',b'e'];let mut phase=[first,b'-',b's',b'p',b'h',b'a',b's',b'e'];let mut input=[0u8;9];let mut ev=[0u8;16];
 let mut surface=match guest_api::Surface::create(128,64){Ok(s)=>s,Err(_)=>stop(3)};let id=surface.id();
 let bg=if first==b'a'{0x1d8059}else{0x16397a};let mark=if first==b'a'{0xce574f}else{0xffb347};
 if surface.fill(0,0,128,64,bg as u32).is_err() || surface.fill(4,4,20,20,mark as u32).is_err() || surface.present().is_err() {stop(4)}
 if fill(id,0,0,0,1,bg)!=(-22i64) as u64 {stop(5)}
 let mut id_bytes=id.to_le_bytes();if file(true,phase.as_mut_ptr(),8,id_bytes.as_mut_ptr(),8)!=8 {stop(6)}
 loop {if file(false,gate.as_mut_ptr(),7,input.as_mut_ptr(),9)!=9 {stop(7)}if input[8]==b'1'||input[8]==b'p' {break}if input[8]!=b'0' {stop(8)}}
 let peer=u64::from_le_bytes([input[0],input[1],input[2],input[3],input[4],input[5],input[6],input[7]]);
 if peer==0 || peer==id {stop(9)}
 if input[8]==b'p' {
  // Former surface ID must not target a new incarnation's surface.
  if present(peer)!=(-116i64) as u64 || surface.destroy().is_err() {stop(10)}
  input[0]=b'p';if file(true,phase.as_mut_ptr(),8,input.as_mut_ptr(),1)!=1 {stop(11)}stop(0)
 }
 // Another live actor's ID is never authority over its surface or events.
 if fill(peer,0,0,1,1,bg)!=(-13i64) as u64 || present(peer)!=(-13i64) as u64 || event(peer,ev.as_mut_ptr())!=(-13i64) as u64 {stop(12)}
 input[0]=b'r';if file(true,phase.as_mut_ptr(),8,input.as_mut_ptr(),1)!=1 {stop(13)}
 // Hold polling until the host has exercised a full event queue.
 loop {if file(false,gate.as_mut_ptr(),7,input.as_mut_ptr(),9)!=9 {stop(21)}if input[8]==b'i' {break}if input[8]!=b'1' {stop(22)}}
 loop {
  let event=match surface.poll_event(){Ok(Some(e))=>e,Ok(None)=>continue,Err(_)=>stop(14)};
  let (x,y,buttons,source)=(event.x,event.y,event.buttons,event.source);
  if source!=(if first==b'a' {0}else{1}) {stop(15)}
  if buttons&1==0 {continue}
  if !(28..=32).contains(&x) || !(23..=27).contains(&y) {stop(16)}break
 }
 input[0]=b'd';if file(true,phase.as_mut_ptr(),8,input.as_mut_ptr(),1)!=1 {stop(17)}
 loop {if file(false,gate.as_mut_ptr(),7,input.as_mut_ptr(),9)!=9 {stop(18)}if input[8]==b'q' {break}if input[8]!=b'i' {stop(19)}}
 if surface.destroy().is_err() {stop(20)}stop(0)
}}
