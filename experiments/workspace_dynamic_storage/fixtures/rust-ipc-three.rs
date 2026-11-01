//! One bounded direct channel shared by a selected pair of three actors.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;8]=[0;8];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn peer()->(u64,u64,u64) {let mut s=0u64;let i:u64;let c:u64;asm!("svc #0x160",inout("x0") s,lateout("x1") i,lateout("x2") c,options(nostack));(s,i,c)}
unsafe fn send(slot:u64,inc:u64,id:u64,p:*const u8)->u64 {let mut r=slot;asm!("svc #0x161",inout("x0") r,in("x1") inc,in("x2") p as u64,in("x3") 1u64,in("x4") id,options(nostack));r}
unsafe fn recv(id:u64,p:*mut u8)->(u64,u64,u64) {let mut n=p as u64;let mut slot=1u64;let mut inc=id;asm!("svc #0x162",inout("x0") n,inout("x1") slot,inout("x2") inc,options(nostack));(n,slot,inc)}
unsafe fn wait(id:u64)->u64 {let mut r=id;asm!("svc #0x163",inout("x0") r,options(nostack));r}
unsafe fn timed(id:u64)->u64 {let mut r=id;asm!("svc #0x164",inout("x0") r,in("x1") 10_000_000_000u64,options(nostack));r}
unsafe fn file(write:bool,name:*mut u8,data:*mut u8)->u64 {loop {let mut r=name as u64;if write {asm!("svc #0x141",inout("x0") r,in("x1") 7u64,in("x2") data as u64,in("x3") 1u64,options(nostack))}else{asm!("svc #0x140",inout("x0") r,in("x1") 7u64,in("x2") data as u64,in("x3") 1u64,options(nostack))}if r!=(-11i64) as u64{return r}}}
unsafe fn gate(name:*mut u8,data:*mut u8,want:u8) {loop {if file(false,name,data)!=1{stop(42)}if read_volatile(data)==want{return}}}
unsafe fn phase(name:*mut u8,data:*mut u8,value:u8) {write_volatile(data,value);if file(true,name,data)!=1{stop(42)}}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let prefix=match actor {0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(42)};
  if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788{stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();if (0..8).any(|i|read_volatile(data.add(i))!=0){stop(42)}
  let mut name=[prefix,b'-',b'i',b'g',b'a',b't',b'e'];let n=name.as_mut_ptr();
  let mut stack=[0xa5u8,0,0x5a];let p=stack.as_mut_ptr().add(1);
  if (p as usize)<0x50003000 || (p as usize)>=0x50004000{stop(42)}
  // First C incarnation starts at 1; C after readmission starts at 4.
  let first=loop {if file(false,n,p)!=1{stop(42)}let v=read_volatile(p);if v==b'1'||(actor==0xc3c3&&v==b'4'){break v}};
  if actor==0xa1a1 {
   if peer().0!=(-107i64) as u64 || send(1,1,1,p)!=(-107i64) as u64 || recv(1,p).0!=(-107i64) as u64 || wait(1)!=(-107i64) as u64 || timed(1)!=(-107i64) as u64 {stop(42)}
   phase(n,p,b'v');gate(n,p,b'4');let (slot,inc,id)=peer();
   if slot!=2||inc==0||id<=1{stop(42)}
   if send(slot,inc,id-1,p)!=(-116i64) as u64 || recv(id-1,p).0!=(-116i64) as u64{stop(42)}
   gate(n,p,b'5');write_volatile(p,b'N');if send(slot,inc,id,p)!=0{stop(42)}
   if wait(id)!=(-107i64) as u64{stop(42)}
  } else if actor==0xb2b2 {
   let (slot,inc,id)=peer();if slot!=2||inc==0||id==0{stop(42)}
   gate(n,p,b'2');write_volatile(p,b'1');if send(slot,inc,id,p)!=0{stop(42)}
   write_volatile(data,b'2');if send(slot,inc,id,data)!=0 || send(slot,inc,id,p)!=(-11i64) as u64{stop(42)}
   phase(n,p,b'f');while peer().0!=(-107i64) as u64{}phase(n,p,b'd');gate(n,p,b'7');
   if peer().0!=(-107i64) as u64 || send(0,1,1,p)!=(-107i64) as u64 || recv(1,p).0!=(-107i64) as u64 || wait(1)!=(-107i64) as u64 || timed(1)!=(-107i64) as u64 {stop(42)}
   phase(n,p,b'v');gate(n,p,b'9');
  } else if first==b'1' {
   let (slot,inc,id)=peer();if slot!=1||inc==0||id==0{stop(42)}
   if timed(id)!=0{stop(42)}gate(n,p,b'3');
   if recv(id,p)!=(1,slot,inc)||read_volatile(p)!=b'1' || recv(id,data)!=(1,slot,inc)||read_volatile(data)!=b'2'{stop(42)}
   if timed(id)!=(-110i64) as u64{stop(42)}phase(n,p,b't');
   // Controller stops this sleeping incarnation.
   if wait(id)!=(-107i64) as u64{stop(42)}
  } else {
   let (slot,inc,id)=peer();if slot!=0||inc==0||id==0{stop(42)}
   if timed(id)!=0 || recv(id,p)!=(1,slot,inc)||read_volatile(p)!=b'N'{stop(42)}
   phase(n,p,b'n');if wait(id)!=(-107i64) as u64{stop(42)}
  }
  if read_volatile(stack.as_ptr())!=0xa5||read_volatile(stack.as_ptr().add(2))!=0x5a{stop(42)}
  stop(0)
 }
}
