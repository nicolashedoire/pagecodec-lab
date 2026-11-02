//! Three ELFs exercise independently allocated indexed user heap pages.
#![no_std]
#![no_main]
#[path="../guest_api.rs"] mod guest_api;
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
const HEAP:u64=0x50004000;
unsafe fn alloc(index:u64)->u64 {let mut x=index;asm!("svc #0x182",inout("x0") x,options(nostack));x}
unsafe fn free(index:u64)->u64 {let mut x=index;asm!("svc #0x183",inout("x0") x,options(nostack));x}
unsafe fn io(write:bool,name:*mut u8,value:*mut u8)->u64 {
 let mut x=name as u64;
 if write {asm!("svc #0x141",inout("x0") x,in("x1") 7u64,in("x2") value as u64,in("x3") 1u64,options(nostack))}
 else {asm!("svc #0x140",inout("x0") x,in("x1") 7u64,in("x2") value as u64,in("x3") 1u64,options(nostack))}
 x
}
unsafe fn empty_put(name:*mut u8,value:*mut u8)->u64 {
 let mut x=name as u64;asm!("svc #0x141",inout("x0") x,in("x1") 7u64,in("x2") value as u64,in("x3") 0u64,options(nostack));x
}
unsafe fn complete(write:bool,name:*mut u8,value:*mut u8)->u64 {
 loop {let x=io(write,name,value);if x!=(-11i64) as u64{return x}}
}
unsafe fn page(index:u64)->*mut u8 {(HEAP+index*4096) as *mut u8}
unsafe fn zero_page(index:u64) {
 let p=page(index).cast::<u64>();for i in 0..512 {if read_volatile(p.add(i))!=0 {stop(42)}}
}
unsafe fn mark(index:u64,marker:u64) {
 let p=page(index);write_volatile(p.cast::<u64>(),marker^index);write_volatile(p.add(4088).cast::<u64>(),!(marker^index));
}
unsafe fn check(marker:u64) {
 for index in 0..8 {let p=page(index);if read_volatile(p.cast::<u64>())!=marker^index || read_volatile(p.add(4088).cast::<u64>())!=!(marker^index) {stop(42)}}
}
unsafe fn name_at(index:u64,first:u8)->*mut u8 {
 let p=page(index).add(4000);for (i,b) in [first,b'-',b'p',b'g',b'a',b't',b'e'].iter().enumerate() {write_volatile(p.add(i),*b)}p
}
unsafe fn name(first:u8)->*mut u8 {name_at(0,first)}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
 unsafe {
  let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',0xc3c3=>b'c',_=>stop(42)};
  if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(42)}
  if alloc(8)!=(-22i64) as u64 || free(8)!=(-22i64) as u64 {stop(42)}
  let marker=actor.wrapping_mul(0x100010001);
  // The stack ends at the first heap page. An accepted empty PUT using that
  // one-past pointer must likewise prevent mapping heap page 0 until consumed.
  let mut stack_name=[first,b'-',b'p',b'g',b'a',b't',b'e'];
  loop {
   if empty_put(stack_name.as_mut_ptr(),HEAP as *mut u8)!=(-11i64) as u64 {stop(42)}
   let result=alloc(0);
   if result==(-16i64) as u64 {break}
   if result!=HEAP || free(0)!=0 {stop(42)}
  }
  loop {let result=empty_put(stack_name.as_mut_ptr(),HEAP as *mut u8);if result==(-11i64) as u64 {continue}if result!=0 {stop(42)}break}
  if alloc(0)!=HEAP {stop(42)}zero_page(0);mark(0,marker);
  let filename=name(first);let one_past=page(1);
  // An accepted zero-length PUT at the end of page 0 pins its address
  // classification. Mapping the previously absent adjacent page must wait.
  loop {
   if empty_put(filename,one_past)!=(-11i64) as u64 {stop(42)}
   let result=alloc(1);
   if result==(-16i64) as u64 {break}
   if result!=HEAP+4096 || free(1)!=0 {stop(42)}
  }
  loop {let result=empty_put(filename,one_past);if result==(-11i64) as u64 {continue}if result!=0 {stop(42)}break}
  if alloc(1)!=HEAP+4096 {stop(42)}zero_page(1);mark(1,marker);
  let value=page(1).add(4000);write_volatile(value,b'0');if complete(true,filename,value)!=1 {stop(42)}
  for i in 2..8 {if alloc(i)!=HEAP+i*4096 {stop(42)}zero_page(i);mark(i,marker)}
  if alloc(0)!=(-17i64) as u64 {stop(42)}
  let mut filename=name(first);let output=page(1).add(4000);
  // Test precise pinning: page 0 holds the name, page 1 the output; page 2
  // is unrelated. A full queue can return -11 without accepting a request.
  loop {
   if io(false,filename,output)!=(-11i64) as u64 {stop(42)}
   let result=free(0);
   if result==(-16i64) as u64 {break}
   if result!=0 || alloc(0)!=HEAP {stop(42)}
   zero_page(0);mark(0,marker);filename=name(first);
  }
  if free(1)!=(-16i64) as u64 || free(2)!=0 || alloc(2)!=HEAP+8192 {stop(42)}
  zero_page(2);mark(2,marker);
  if complete(false,filename,output)!=1 {stop(42)}
  write_volatile(output,b'r');if complete(true,filename,output)!=1 {stop(42)}
  loop {if complete(false,filename,output)!=1 {stop(42)}check(marker);match read_volatile(output){b'r'=>{},b'f'=>break,_=>stop(42)}}
  for i in 0..8 {if free(i)!=0 {stop(42)}}
  if free(0)!=(-2i64) as u64 {stop(42)}
  // A zero-length source at the start of an existing page remains valid even
  // when the preceding page is absent; it must not be rejected as one-past.
  if alloc(1)!=HEAP+4096 {stop(42)}zero_page(1);
  let filename=name_at(1,first);
  loop {let result=empty_put(filename,page(1));if result==(-11i64) as u64 {continue}if result!=0 {stop(42)}break}
  if free(1)!=0 {stop(42)}
  for i in (0..8).rev() {if alloc(i)!=HEAP+i*4096 {stop(42)}zero_page(i);mark(i,marker)}
  let filename=name(first);let output=page(1).add(4000);
  write_volatile(output,b'd');if complete(true,filename,output)!=1 {stop(42)}
  loop {if complete(false,filename,output)!=1 {stop(42)}check(marker);match read_volatile(output){b'd'=>{},b'q'=>break,_=>stop(42)}}
  for i in 0..8 {if free(i)!=0 {stop(42)}}
  if !matches!(guest_api::HeapPage::allocate_index(8),Err(-22)) {stop(42)}
  let mut owned=match guest_api::HeapPage::allocate_index(7) {Ok(page)=>page,Err(_)=>stop(42)};
  zero_page(7);
  let bytes=owned.as_mut_slice();let (front,tail)=bytes.split_at_mut(4016);
  for (i,b) in [first,b'-',b'p',b'g',b'a',b't',b'e'].iter().enumerate() {front[4000+i]=*b}
  let name=core::str::from_utf8_unchecked(&front[4000..4007]);let output=&mut tail[..1];
  if guest_api::read_file(name,output)!=Ok(1) || output[0]!=b'q' || guest_api::write_file(name,output)!=Ok(1) {stop(42)}
  if owned.release().is_err() {stop(42)}stop(0)
 }
}
