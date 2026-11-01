//! Deterministic direct IPC queue-full and close/reopen exercise.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;8]=[0;8];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn peer()->(u64,u64,u64) {
 let mut slot=0u64;let inc:u64;let channel:u64;
 asm!("svc #0x160",inout("x0") slot,lateout("x1") inc,lateout("x2") channel,options(nostack));(slot,inc,channel)
}
unsafe fn wait_peer()->(u64,u64,u64) {
 loop {let p=peer();if p.0!=(-107i64) as u64 {return p}}
}
unsafe fn wait_closed() {while peer().0!=(-107i64) as u64 {}}
unsafe fn send_once(slot:u64,inc:u64,channel:u64,buf:*const u8,len:u64)->u64 {
 let mut r=slot;asm!("svc #0x161",inout("x0") r,in("x1") inc,in("x2") buf as u64,in("x3") len,in("x4") channel,options(nostack));r
}
unsafe fn recv_once(channel:u64,buf:*mut u8,cap:u64)->(u64,u64,u64) {
 let mut n=buf as u64;let mut s=cap;let mut inc=channel;
 asm!("svc #0x162",inout("x0") n,inout("x1") s,inout("x2") inc,options(nostack));(n,s,inc)
}
unsafe fn receive(channel:u64,buf:*mut u8)->(u64,u64,u64) {
 loop {let r=recv_once(channel,buf,1);if r.0!=(-11i64) as u64 {return r}}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  if actor!=0xa1a1 && actor!=0xb2b2 || read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 {stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();if (0..8).any(|i|read_volatile(data.add(i))!=0) {stop(42)}
  let mut stack=[0xa5u8,0,0x5a];let p=stack.as_mut_ptr().add(1);
  if (p as usize)<0x50003000 || (p as usize)>=0x50004000 {stop(42)}
  let (slot,inc,first)=wait_peer();if slot>1 || inc==0 || first==0 {stop(42)}
  if actor==0xa1a1 {
   let r=receive(first,p);if r!=(1,slot,inc) || read_volatile(p)!=b'R' {stop(42)}
   write_volatile(p,b'1');if send_once(slot,inc,first,p,1)!=0 {stop(42)}
   write_volatile(data,b'2');if send_once(slot,inc,first,data,1)!=0 {stop(42)}
   if send_once(slot,inc,first,p,1)!=(-11i64) as u64 {stop(42)}
  } else {
   write_volatile(p,b'R');if send_once(slot,inc,first,p,1)!=0 {stop(42)}
  }
  wait_closed();
  let (slot2,inc2,second)=wait_peer();if slot2!=slot || inc2!=inc || second==first || second==0 {stop(42)}
  if send_once(slot,inc,first,p,1)!=(-116i64) as u64 || recv_once(first,p,1).0!=(-116i64) as u64 {stop(42)}
  if actor==0xb2b2 {
   if recv_once(second,p,1).0!=(-11i64) as u64 {stop(42)}
   write_volatile(p,b'S');if send_once(slot,inc,second,p,1)!=0 {stop(42)}
  } else {
   let r=receive(second,p);if r!=(1,slot,inc) || read_volatile(p)!=b'S' {stop(42)}
  }
  wait_closed();
  if read_volatile(stack.as_ptr())!=0xa5 || read_volatile(stack.as_ptr().add(2))!=0x5a {stop(42)}
  stop(0)
 }
}
