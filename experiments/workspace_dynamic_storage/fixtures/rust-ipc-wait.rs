//! True blocking IPC wait and wake, exercised by two private ELF tasks.
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
unsafe fn wait_peer()->(u64,u64,u64) {loop {let p=peer();if p.0!=(-107i64) as u64{return p}}}
unsafe fn wait(channel:u64)->u64 {let mut r=channel;asm!("svc #0x163",inout("x0") r,options(nostack));r}
unsafe fn send(slot:u64,inc:u64,channel:u64,buf:*const u8,len:u64)->u64 {
 let mut r=slot;asm!("svc #0x161",inout("x0") r,in("x1") inc,in("x2") buf as u64,in("x3") len,in("x4") channel,options(nostack));r
}
unsafe fn receive(channel:u64,buf:*mut u8,cap:u64)->(u64,u64,u64) {
 let mut n=buf as u64;let mut slot=cap;let mut inc=channel;
 asm!("svc #0x162",inout("x0") n,inout("x1") slot,inout("x2") inc,options(nostack));(n,slot,inc)
}
unsafe fn gate() {
 // Host-only control signal. The IPC payload itself never uses VFS.
 let name=*b"a-wgate";let mut byte=[0u8];
 loop {
  let mut r=name.as_ptr() as u64;
  asm!("svc #0x140",inout("x0") r,in("x1") name.len() as u64,in("x2") byte.as_mut_ptr() as u64,in("x3") 1u64,options(nostack));
  if r==1 {if byte[0]==b'1' {return}if byte[0]!=b'0' {stop(42)}}else if r!=(-11i64) as u64 {stop(42)}
 }
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  if (actor!=0xa1a1 && actor!=0xb2b2) || read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 {stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();if (0..8).any(|i|read_volatile(data.add(i))!=0){stop(42)}
  let mut stack=[0xa5u8,0,0x5a];let p=stack.as_mut_ptr().add(1);
  if (p as usize)<0x50003000 || (p as usize)>=0x50004000 {stop(42)}
  let (slot,inc,channel)=wait_peer();if slot>1 || inc==0 || channel==0 {stop(42)}
  if wait(channel.wrapping_add(1))!=(-116i64) as u64 {stop(42)}
  if actor==0xa1a1 {
   gate();write_volatile(p,b'W');
   let sent=send(slot,inc,channel,p,1);
   if sent==(-107i64) as u64 {stop(0)}
   if sent!=0 {stop(42)}
   // A file request can remain outstanding while this actor sleeps on IPC.
   let name=*b"a-wgate";let mut byte=[0u8];
   let mut r=name.as_ptr() as u64;
   asm!("svc #0x140",inout("x0") r,in("x1") name.len() as u64,in("x2") byte.as_mut_ptr() as u64,in("x3") 1u64,options(nostack));
   if r!=(-11i64) as u64 || wait(channel)!=(-107i64) as u64 {stop(42)}
   loop {
    let mut result=name.as_ptr() as u64;
    asm!("svc #0x140",inout("x0") result,in("x1") name.len() as u64,in("x2") byte.as_mut_ptr() as u64,in("x3") 1u64,options(nostack));
    if result==1 {if byte[0]!=b'1' {stop(42)}break}
    if result!=(-11i64) as u64 {stop(42)}
   }
  } else {
   if wait(channel)!=0 {stop(42)}
   if receive(channel,data,1)!=(1,slot,inc) || read_volatile(data)!=b'W' {stop(42)}
   if wait(channel)!=(-107i64) as u64 {stop(42)}
  }
  if read_volatile(stack.as_ptr())!=0xa5 || read_volatile(stack.as_ptr().add(2))!=0x5a {stop(42)}
  stop(0)
 }
}
