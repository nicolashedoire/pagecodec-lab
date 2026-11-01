//! Two simultaneous, distinct direct channels across three isolated tasks.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_unaligned,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;32]=[0;32];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
#[inline(never)] unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
#[inline(never)] unsafe fn current()->u64 {let mut r=0u64;asm!("svc #0x160",inout("x0") r,options(nostack));r}
#[inline(never)] unsafe fn peer(id:u64)->(u64,u64,u64) {let mut s=id;let i:u64;let c:u64;asm!("svc #0x165",inout("x0") s,lateout("x1") i,lateout("x2") c,options(nostack));(s,i,c)}
#[inline(never)] unsafe fn send(slot:u64,inc:u64,id:u64,p:*const u8)->u64 {let mut r=slot;asm!("svc #0x161",inout("x0") r,in("x1") inc,in("x2") p as u64,in("x3") 1u64,in("x4") id,options(nostack));r}
#[inline(never)] unsafe fn wait(id:u64)->u64 {let mut r=id;asm!("svc #0x163",inout("x0") r,options(nostack));r}
#[inline(never)] unsafe fn timed(id:u64)->u64 {let mut r=id;asm!("svc #0x164",inout("x0") r,in("x1") 1_000_000_000u64,options(nostack));r}
#[inline(never)] unsafe fn recv(id:u64,p:*mut u8)->(u64,u64,u64) {let mut n=p as u64;let mut s=1u64;let mut inc=id;asm!("svc #0x162",inout("x0") n,inout("x1") s,inout("x2") inc,options(nostack));(n,s,inc)}
#[inline(never)] unsafe fn receive(id:u64,p:*mut u8,slot:u64,inc:u64,byte:u8) {if wait(id)!=0||recv(id,p)!=(1,slot,inc)||read_volatile(p)!=byte{stop(42)}}
#[inline(never)] unsafe fn file(name:*mut u8,data:*mut u8)->u64 {loop{let mut r=name as u64;asm!("svc #0x140",inout("x0") r,in("x1") 7u64,in("x2") data as u64,in("x3") 32u64,options(nostack));if r!=(-11i64) as u64{return r}}}
#[inline(never)] unsafe fn ids(name:*mut u8,data:*mut u8,previous:u64)->(u64,u64,u64) {
 loop {
  let n=file(name,data);if n==1&&read_volatile(data)==b'0'{continue}if n!=17{stop(42)}
  let ab=u64::from_le(read_unaligned(data.cast::<u64>()));let ac=u64::from_le(read_unaligned(data.add(8).cast::<u64>()));let stage=read_volatile(data.add(16)) as u64;
  if ab==0||ac==0||ab==ac||(stage<1||stage>4){stop(42)}
  if ab!=previous{return (ab,ac,stage)}
 }
}
#[inline(never)] unsafe fn phase(name:*mut u8,p:*mut u8,value:u8) {
 write_volatile(p,value);
 loop {let mut r=name as u64;asm!("svc #0x141",inout("x0") r,in("x1") 7u64,in("x2") p as u64,in("x3") 1u64,options(nostack));if r==1{return}if r!=(-11i64) as u64{stop(42)}}
}
#[inline(never)] unsafe fn gate_stage(name:*mut u8,data:*mut u8,want:u8) {
 loop {let n=file(name,data);if n==1&&read_volatile(data)==b'r'{continue}if n!=17{stop(42)}if read_volatile(data.add(16))==want{return}}
}
#[inline(never)] unsafe fn outsider(foreign:u64,own:Link,p:*mut u8) {
 if peer(foreign).0!=(-116i64) as u64||send(own.slot,own.inc,foreign,p)!=(-116i64) as u64||recv(foreign,p).0!=(-116i64) as u64||wait(foreign)!=(-116i64) as u64||timed(foreign)!=(-116i64) as u64{stop(42)}
}
#[derive(Clone,Copy)] struct Link{slot:u64,inc:u64,id:u64}
#[inline(never)] unsafe fn link(id:u64,expected:u64)->Link {let (slot,inc,got)=peer(id);if slot!=expected||inc==0||got!=id{stop(42)}Link{slot,inc,id}}
#[inline(never)] unsafe fn emit(l:Link,p:*mut u8,byte:u8) {write_volatile(p,byte);if send(l.slot,l.inc,l.id,p)!=0{stop(42)}}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let prefix=match actor{0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(42)};
  if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788{stop(42)}
  let data=addr_of_mut!(BUF).cast::<u8>();if (0..32).any(|i|read_volatile(data.add(i))!=0){stop(42)}
  let mut name=[prefix,b'-',b'm',b'g',b'a',b't',b'e'];let n=name.as_mut_ptr();
  let mut stack=[0xa5u8,0,0x5a];let p=stack.as_mut_ptr().add(1);
  if (p as usize)<0x50003000||(p as usize)>=0x50004000{stop(42)}
  let (ab,ac,stage)=ids(n,data,0);
  if actor==0xa1a1 {
   if (stage!=1&&stage!=3&&stage!=4)||current()!=(-76i64) as u64{stop(42)}
   let b=link(ab,1);let c=link(ac,2);
   if stage==4 {receive(ab,p,b.slot,b.inc,b'b');while peer(ab).0!=(-107i64) as u64{}stop(0)}
   emit(b,p,b'b');emit(c,p,b'c');
   receive(ab,p,b.slot,b.inc,b'B');receive(ac,p,c.slot,c.inc,b'C');
   if stage==3 {if wait(ab)!=(-107i64) as u64{stop(42)}stop(0)}
   while peer(ab).0!=(-116i64) as u64{}
   emit(c,p,b'd');receive(ac,p,c.slot,c.inc,b'D');
   let (new_ab,new_ac,new_stage)=ids(n,data,ab);if new_ac!=ac||new_stage!=2{stop(42)}
   let b2=link(new_ab,1);outsider(ab,c,p);
   emit(b2,p,b'e');emit(c,p,b'f');receive(new_ab,p,b2.slot,b2.inc,b'E');receive(ac,p,c.slot,c.inc,b'F');
   while peer(ac).0!=(-107i64) as u64{}
  } else if actor==0xb2b2 {
   let b=link(ab,0);if current()!=0{stop(42)}outsider(ac,b,p);
   if stage==4 {phase(n,p,b'r');gate_stage(n,data,5);emit(b,p,b'b');while peer(ab).0!=(-107i64) as u64{}stop(0)}
   if stage==1||stage==3 {receive(ab,p,b.slot,b.inc,b'b');emit(b,p,b'B');while peer(ab).0!=(-107i64) as u64{}if stage==3{stop(0)}loop{core::hint::spin_loop()}}
   else {receive(ab,p,b.slot,b.inc,b'e');emit(b,p,b'E');while peer(ab).0!=(-107i64) as u64{}}
  } else {
   if stage!=1&&stage!=3&&stage!=4{stop(42)}let c=link(ac,0);if current()!=0{stop(42)}outsider(ab,c,p);
   if stage==4 {phase(n,p,b'r');gate_stage(n,data,5);emit(c,p,b'c');while peer(ac).0!=(-107i64) as u64{}stop(0)}
   receive(ac,p,c.slot,c.inc,b'c');emit(c,p,b'C');if stage==3 {while peer(ac).0!=(-107i64) as u64{}stop(0)}receive(ac,p,c.slot,c.inc,b'd');emit(c,p,b'D');
   receive(ac,p,c.slot,c.inc,b'f');emit(c,p,b'F');while peer(ac).0!=(-107i64) as u64{}
  }
  if read_volatile(stack.as_ptr())!=0xa5||read_volatile(stack.as_ptr().add(2))!=0x5a{stop(42)}
  stop(0)
 }
}
