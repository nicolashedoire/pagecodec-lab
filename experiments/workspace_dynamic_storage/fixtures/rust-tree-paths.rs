//! Real long-path tree operations alongside unchanged short flat/document ABI.
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
unsafe fn tree_op_once(path:*mut u8,len:u64,g:u64)->u64 {
 let mut result=path as u64;asm!("svc #0x14a",inout("x0") result,in("x1") len,in("x2") g,options(nostack));result
}
unsafe fn tree_op(path:*mut u8,len:u64,g:u64)->u64 {
 loop {let result=tree_op_once(path,len,g);if result!=(-11i64) as u64{return result}}
}
unsafe fn tree_read(path:*mut u8,len:u64,out:*mut u8,cap:u64,g:u64)->u64 {
 loop {let mut result=path as u64;asm!("svc #0x144",inout("x0") result,in("x1") len,in("x2") out as u64,in("x3") cap,in("x4") 0u64,in("x5") g,options(nostack));if result!=(-11i64) as u64{return result}}
}
unsafe fn doc_read(key:*mut u8,n:u64,out:*mut u8,binding:u64)->u64 {
 loop {let mut result=key as u64;asm!("svc #0x190",inout("x0") result,in("x1") n,in("x2") out as u64,in("x3") 1u64,in("x4") binding,options(nostack));if result!=(-11i64) as u64{return result}}
}
fn value(first:u8,index:usize)->u8 {((index.wrapping_mul(19)^usize::from(first))&255) as u8}
fn sum(first:u8)->u64 {let mut h=0xcbf29ce484222325u64;for i in 0..65 {h=(h^u64::from(value(first,i))).wrapping_mul(0x100000001b3)}h}
unsafe fn next(old:u64)->u64 {match guest_api::tree_generation(){Ok(Some(v)) if v==old+1=>v,_=>stop(2)}}
unsafe fn write_tree(path:&str,first:u8,g:u64,data:&mut [u8;32]) {
 let mut u=match guest_api::TreeUpload::begin(path,65,sum(first),g){Ok(u)=>u,Err(_)=>stop(3)};
 for offset in (0..65).step_by(32) {let n=core::cmp::min(65-offset,32);for i in 0..n {data[i]=value(first,offset+i)}if u.append(&data[..n])!=Ok(n){stop(4)}}
 if u.commit()!=Ok(()){stop(5)}
}
#[no_mangle] pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {unsafe {
 let first=match actor {0xa1a1=>b'a',0xb2b2=>b'b',_=>stop(1)};
 if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 || read_volatile(addr_of_mut!(ZERO).cast::<u64>())!=0 {stop(6)}
 let mut gate=[first,b'-',b'p',b'g',b'a',b't',b'e'];let mut phase=[first,b'-',b'p',b'p',b'h',b'a',b's',b'e'];
 let mut data=[0u8;32];let mut token=[0u8;9];token[0]=b'r';if file(true,phase.as_mut_ptr(),8,token.as_mut_ptr(),1)!=1 {stop(7)}
 loop {if file(false,gate.as_mut_ptr(),7,token.as_mut_ptr(),9)!=9 {stop(8)}if token[8]==b'1'{break}if token[8]!=b'0' {stop(9)}}
 let binding=u64::from_le_bytes([token[0],token[1],token[2],token[3],token[4],token[5],token[6],token[7]]);if binding==0 {stop(10)}
 let mut g=match guest_api::tree_generation(){Ok(Some(g))=>g,_=>stop(11)};
 // Flat and document key ABIs remain capped at 16 despite the tree expansion.
 let mut old_key=[b'a';17];old_key[1]=b'-';if file(false,old_key.as_mut_ptr(),17,data.as_mut_ptr(),1)!=(-22i64) as u64 {stop(12)}
 if doc_read(old_key.as_mut_ptr(),17,data.as_mut_ptr(),binding)!=(-22i64) as u64 {stop(13)}
 let mut dir=[b'k';256];dir[0]=b'/';dir[1]=first;dir[2]=b'/';
 if tree_op(dir.as_mut_ptr(),256,g)!=(-22i64) as u64 {stop(14)}
 if tree_read(dir.as_mut_ptr(),256,data.as_mut_ptr(),1,g)!=(-22i64) as u64 {stop(15)}
 if tree_op(0x50000ff0u64 as *mut u8,32,g)!=(-14i64) as u64 {stop(16)}
 let mut foreign=dir;foreign[1]=if first==b'a' {b'b'}else{b'a'};
 if tree_op(foreign.as_mut_ptr(),255,g)!=(-13i64) as u64 {stop(17)}
 let dir_path=core::str::from_utf8_unchecked(&dir[..255]);
 if first==b'a' {
  let mut owned=match guest_api::HeapPage::allocate_index(0){Ok(p)=>p,Err(_)=>stop(18)};
  let path=owned.as_mut_slice().as_mut_ptr();for i in 0..255 {path.add(i).write(dir[i])}
  if tree_op_once(path,255,g)!=(-11i64) as u64 {stop(31)}
  path.add(254).write(b'z');if tree_op_once(path,255,g)!=(-22i64) as u64 {stop(32)}path.add(254).write(b'k');
  owned=match owned.release() {Err((-16,page))=>page,_=>stop(33)};
  if tree_op(path,255,g)!=0 {stop(34)}if owned.release().is_err() {stop(35)}
 } else if guest_api::tree_mkdir(dir_path,g)!=Ok(()) {stop(18)}
 g=next(g);
 let mut root=[b'/',first];let root_path=core::str::from_utf8_unchecked(&root);
 let mut offset=0usize;
 while offset<252 {data.fill(0xee);let n=match guest_api::tree_list_fragment(root_path,0,&mut data,g,offset){Ok(n)=>n,Err(_)=>stop(19)};
  let take=core::cmp::min(31,252-offset);if n!=take+1 || data[0]!=(if offset+take<252 {0x80}else{0}) || data[1..n].iter().any(|x|*x!=b'k') {stop(20)}offset+=take;
 }
 if guest_api::tree_list_fragment(root_path,1,&mut data,g,0)!=Ok(0) {stop(21)}
 if guest_api::tree_read_at(dir_path,&mut data,0,g)!=Err(-21) {stop(22)}
 let mut file_path=[b'f';33];file_path[0]=b'/';file_path[1]=first;file_path[2]=b'/';let file_string=core::str::from_utf8_unchecked(&file_path);
 write_tree(file_string,first,g,&mut data);g=next(g);
 for offset in (0..65).step_by(32) {let count=core::cmp::min(65-offset,32);data.fill(0xee);
  if guest_api::tree_read_at(file_string,&mut data,offset,g)!=Ok(count) {stop(23)}for i in 0..count {if data[i]!=value(first,offset+i){stop(24)}}
  for x in &data[count..] {if *x!=0xee {stop(25)}}
 }
 if guest_api::tree_remove(dir_path,g)!=Ok(()) {stop(26)}g=next(g);
 if guest_api::tree_remove(file_string,g)!=Ok(()) {stop(27)}g=next(g);
 let mut keep=[b'p';33];keep[0]=b'/';keep[1]=first;keep[2]=b'/';let keep_string=core::str::from_utf8_unchecked(&keep);
 write_tree(keep_string,first,g,&mut data);let _=next(g);
 token[0]=b'd';if file(true,phase.as_mut_ptr(),8,token.as_mut_ptr(),1)!=1 {stop(28)}
 loop {if file(false,gate.as_mut_ptr(),7,token.as_mut_ptr(),9)!=9 {stop(29)}if token[8]==b'q' {break}if token[8]!=b'1' {stop(30)}}stop(0)
}}
