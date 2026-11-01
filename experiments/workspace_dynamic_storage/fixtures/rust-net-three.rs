//! Strict grant-bound network SVCs with a selected pair of three tasks.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;8]=[0;8];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
#[derive(Clone,Copy)] struct Link {port:u64,peer:u64,inc:u64,grant:u64}
unsafe fn info()->Link {let mut port=0u64;let peer:u64;let inc:u64;let grant:u64;asm!("svc #0x174",inout("x0") port,lateout("x1") peer,lateout("x2") inc,lateout("x3") grant,options(nostack));Link{port,peer,inc,grant}}
unsafe fn send(link:Link,p:*const u8)->u64 {let mut r=p as u64;asm!("svc #0x172",inout("x0") r,in("x1") 1u64,in("x2") link.grant,in("x3") link.peer,in("x4") link.inc,options(nostack));r}
unsafe fn recv(link:Link,p:*mut u8)->(u64,u64,u64) {let mut n=p as u64;let mut slot=1u64;let mut inc=link.grant;asm!("svc #0x173",inout("x0") n,inout("x1") slot,inout("x2") inc,in("x3") link.peer,in("x4") link.inc,options(nostack));(n,slot,inc)}
unsafe fn legacy(p:*mut u8)->(u64,u64) {let mut s=p as u64;asm!("svc #0x170",inout("x0") s,in("x1") 1u64,options(nostack));let mut r=p as u64;asm!("svc #0x171",inout("x0") r,in("x1") 1u64,options(nostack));(s,r)}
unsafe fn receive(link:Link,p:*mut u8)->(u64,u64,u64) {loop{let r=recv(link,p);if r.0!=(-11i64) as u64{return r}}}
unsafe fn file(write:bool,name:*mut u8,data:*mut u8)->u64 {loop{let mut r=name as u64;if write {asm!("svc #0x141",inout("x0") r,in("x1") 7u64,in("x2") data as u64,in("x3") 1u64,options(nostack))}else{asm!("svc #0x140",inout("x0") r,in("x1") 7u64,in("x2") data as u64,in("x3") 1u64,options(nostack))}if r!=(-11i64) as u64{return r}}}
unsafe fn gate(name:*mut u8,data:*mut u8,want:u8) {loop{if file(false,name,data)!=1{stop(42)}if read_volatile(data)==want{return}}}
unsafe fn phase(name:*mut u8,data:*mut u8,value:u8) {write_volatile(data,value);if file(true,name,data)!=1{stop(42)}}
unsafe fn outsider(p:*mut u8) {let l=Link{port:0,peer:0,inc:1,grant:1};if info().port!=(-13i64) as u64 || send(l,p)!=(-13i64) as u64 || recv(l,p).0!=(-13i64) as u64{stop(42)}}
unsafe fn response(link:Link,p:*mut u8,byte:u8) {let r=receive(link,p);if r!=(1,link.peer,link.inc)||read_volatile(p)!=byte{stop(42)}}
unsafe fn member(p:*mut u8,port:u64,peer:u64)->Link {let l=info();if l.port!=port||l.peer!=peer||l.inc==0||l.grant==0||legacy(p)!=((-38i64) as u64,(-38i64) as u64){stop(42)}l}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let prefix=match actor {0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(42)};
  if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788{stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();if (0..8).any(|i|read_volatile(data.add(i))!=0){stop(42)}
  let mut name=[prefix,b'-',b'n',b'g',b'a',b't',b'e'];let n=name.as_mut_ptr();
  let mut stack=[0xa5u8,0,0x5a];let p=stack.as_mut_ptr().add(1);
  if (p as usize)<0x50003000 || (p as usize)>=0x50004000{stop(42)}
  let first=loop{if file(false,n,p)!=1{stop(42)}let v=read_volatile(p);if v==b'1'||(actor==0xc3c3&&v==b'4'){break v}};
  if actor==0xa1a1 {
   outsider(p);phase(n,p,b'v');gate(n,p,b'4');let link=member(p,0,2);
   // The previous grant and previous C incarnation are both stale.
   let old_grant=Link{grant:link.grant-1,..link};let old_inc=Link{inc:link.inc-1,..link};
   if send(old_grant,p)!=(-116i64) as u64||recv(old_grant,p).0!=(-116i64) as u64||send(old_inc,p)!=(-116i64) as u64||recv(old_inc,p).0!=(-116i64) as u64{stop(42)}
   write_volatile(p,b'A');if send(link,p)!=0{stop(42)}response(link,data,b'C');phase(n,p,b'n');gate(n,p,b'9');
  } else if actor==0xb2b2 {
   let link=member(p,0,2);gate(n,p,b'2');write_volatile(p,b'B');if send(link,p)!=0{stop(42)}response(link,data,b'C');phase(n,p,b'p');
   gate(n,p,b'3');write_volatile(p,b'X');if send(link,p)!=0{stop(42)}phase(n,p,b's');
   gate(n,p,b'7');outsider(p);phase(n,p,b'v');gate(n,p,b'9');
  } else if first==b'1' {
   let link=member(p,1,1);response(link,p,b'B');write_volatile(data,b'C');if send(link,data)!=0{stop(42)}phase(n,p,b'p');gate(n,p,b'9');
  } else {
   let link=member(p,1,0);response(link,p,b'A');write_volatile(data,b'C');if send(link,data)!=0{stop(42)}phase(n,p,b'n');gate(n,p,b'9');
  }
  if read_volatile(stack.as_ptr())!=0xa5||read_volatile(stack.as_ptr().add(2))!=0x5a{stop(42)}
  stop(0)
 }
}
