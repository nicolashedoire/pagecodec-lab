//! Bounded private-buffer round trip over the guest's isolated VirtIO NICs.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;8]=[0;8];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn send_once(buf:*const u8,len:u64)->u64 {
 let mut r=buf as u64;asm!("svc #0x170",inout("x0") r,in("x1") len,options(nostack));r
}
unsafe fn recv_once(buf:*mut u8,cap:u64)->(u64,u64,u64) {
 let mut n=buf as u64;let mut source=cap;let incarnation:u64;
 asm!("svc #0x171",inout("x0") n,inout("x1") source,lateout("x2") incarnation,options(nostack));(n,source,incarnation)
}
unsafe fn send(buf:*const u8,len:u64) {
 loop {let r=send_once(buf,len);if r==0{return}if r!=(-11i64) as u64 {stop(42)}}
}
unsafe fn receive(buf:*mut u8,cap:u64)->(u64,u64,u64) {
 loop {let r=recv_once(buf,cap);if r.0!=(-11i64) as u64 {return r}}
}
#[no_mangle]
pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  if actor!=0xa1a1 && actor!=0xb2b2 || read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 {stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();
  if (0..8).any(|i|read_volatile(data.add(i))!=0) {stop(42)}
  let mut stack=[0xa5u8,0,0,0,0,0x5a];let local=stack.as_mut_ptr().add(1);
  if (local as usize)<0x50003000 || (local as usize)+4>0x50004000 {stop(42)}
  // Missing network returns -19, including when no other task is active.
  loop {let r=send_once(local,0);if r==(-19i64) as u64 {stop(0)}if r==(-107i64) as u64 {continue}if r!=(-22i64) as u64 {stop(42)}break}
  if send_once(local,65)!=(-22i64) as u64 || send_once(0x50003fffusize as *const u8,2)!=(-14i64) as u64 || recv_once(local,0).0!=(-22i64) as u64 {stop(42)}
  if actor==0xa1a1 {
   for (i,b) in b"ping".iter().enumerate(){write_volatile(local.add(i),*b)}
   send(local,4);
   write_volatile(data,0x5a);
   if receive(data,1).0!=(-34i64) as u64 || read_volatile(data)!=0x5a {stop(42)}
   let reply=receive(data,4);
   if reply.0!=4 || reply.1!=1 || reply.2==0 {stop(42)}
   for (i,b) in b"pong".iter().enumerate(){if read_volatile(data.add(i))!=*b {stop(42)}}
  } else {
   write_volatile(local,0x5a);
   if receive(local,1).0!=(-34i64) as u64 || read_volatile(local)!=0x5a {stop(42)}
   let request=receive(local,4);
   if request.0!=4 || request.1!=0 || request.2==0 {stop(42)}
   for (i,b) in b"ping".iter().enumerate(){if read_volatile(local.add(i))!=*b {stop(42)}}
   for (i,b) in b"pong".iter().enumerate(){write_volatile(data.add(i),*b)}
   send(data,4);
   // A only exits after receiving the reply; keep the peer alive until then.
   loop {let r=recv_once(local,4).0;if r==(-107i64) as u64 {break}if r!=(-11i64) as u64 {stop(42)}}
  }
  if read_volatile(stack.as_ptr())!=0xa5 || read_volatile(stack.as_ptr().add(5))!=0x5a {stop(42)}
  stop(0)
 }
}
