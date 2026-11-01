//! Freestanding Rust ELF with private .data, .bss and stack buffers.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};

#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut ZERO:[u64;4]=[0;4];
#[used] static mut NAME:[u8;6]=[0;6];
#[used] static mut VALUE:[u8;1]=[0;1];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}

unsafe fn terminate(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn once<const WRITE:bool>(name:*mut u8,value:*mut u8,len:u64)->u64 {
    let mut result=name as u64;
    if WRITE {asm!("svc #0x141",inout("x0") result,in("x1") 6u64,in("x2") value as u64,in("x3") len,options(nostack))}
    else {asm!("svc #0x140",inout("x0") result,in("x1") 6u64,in("x2") value as u64,in("x3") len,options(nostack))}
    result
}
unsafe fn file_op<const WRITE:bool>(name:*mut u8,value:*mut u8,len:u64)->u64 {
    loop {let r=once::<WRITE>(name,value,len);if r!=(-11i64) as u64 {return r}}
}

#[no_mangle]
pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
    unsafe {
        let data=addr_of_mut!(INITIAL);let zero=addr_of_mut!(ZERO).cast::<u64>();
        let name=addr_of_mut!(NAME).cast::<u8>();let value=addr_of_mut!(VALUE).cast::<u8>();
        let mut stack_word=0u64;let stack=addr_of_mut!(stack_word);
        let mut stack_name=[0u8;6];let local_name=stack_name.as_mut_ptr();
        let mut stack_value=[0xa5u8,0,0x5a];let local_value=stack_value.as_mut_ptr().add(1);
        if actor!=0xa1a1 && actor!=0xb2b2 {terminate(42)}
        if read_volatile(data)!=0x1122_3344_5566_7788 || (0..4).any(|i|read_volatile(zero.add(i))!=0) {terminate(42)}
        if (stack as usize)<0x50003000 || (stack as usize)>=0x50004000 || (local_name as usize)<0x50003000 || (local_name as usize)+6>0x50004000 {terminate(42)}
        write_volatile(data,actor);write_volatile(zero,actor^0xff);write_volatile(stack,!actor);
        let first=if actor==0xa1a1 {b'a'}else{b'b'};
        for (i,byte) in [first,b'-',b'r',b'u',b's',b't'].into_iter().enumerate() {write_volatile(name.add(i),byte);write_volatile(local_name.add(i),byte)}
        // PUT zero bytes with a pointer exactly one byte beyond the stack.
        if file_op::<true>(name,0x50004000usize as *mut u8,0)!=0 {terminate(42)}
        write_volatile(local_value,b'R');
        if file_op::<true>(name,local_value,1)!=1 {terminate(42)}
        write_volatile(value,b'R');
        if file_op::<true>(local_name,value,1)!=1 {terminate(42)}
        // All four name/destination combinations use data and stack buffers.
        if file_op::<false>(name,value,1)!=1 || read_volatile(value)!=b'R' {terminate(42)}
        if file_op::<false>(local_name,value,1)!=1 || read_volatile(value)!=b'R' {terminate(42)}
        if file_op::<false>(name,local_value,1)!=1 || read_volatile(local_value)!=b'R' {terminate(42)}
        if file_op::<false>(local_name,local_value,1)!=1 || read_volatile(local_value)!=b'R' {terminate(42)}
        if read_volatile(local_value.sub(1))!=0xa5 || read_volatile(local_value.add(1))!=0x5a {terminate(42)}
        // Snapshot the PUT source, alter source and name before polling, then
        // restore the name: changed-name polls must fail without losing it.
        write_volatile(local_value,b'S');
        if once::<true>(local_name,local_value,1)!=(-11i64) as u64 {terminate(42)}
        write_volatile(local_value,b'X');write_volatile(local_name.add(2),b'o');
        if once::<true>(local_name,local_value,1)!=(-22i64) as u64 {terminate(42)}
        write_volatile(local_name.add(2),b'r');
        if file_op::<true>(local_name,local_value,1)!=1 {terminate(42)}
        if file_op::<false>(name,value,1)!=1 || read_volatile(value)!=b'S' {terminate(42)}
        write_volatile(value,b'R');if file_op::<true>(name,value,1)!=1 {terminate(42)}
        // GET zero capacity and ranges crossing Data/Stack page boundaries.
        if once::<false>(name,value,0)!=(-22i64) as u64 {terminate(42)}
        if once::<false>(name,0x50003fffusize as *mut u8,2)!=(-14i64) as u64 {terminate(42)}
        if once::<true>(name,0x50000fffusize as *mut u8,2)!=(-14i64) as u64 {terminate(42)}
        if once::<false>(0x50003fffusize as *mut u8,value,1)!=(-14i64) as u64 {terminate(42)}
        if read_volatile(local_value.sub(1))!=0xa5 || read_volatile(local_value.add(1))!=0x5a {terminate(42)}
        for (i,byte) in [first,b'-',b'g',b'a',b't',b'e'].into_iter().enumerate() {write_volatile(name.add(i),byte)}
        loop {
            if file_op::<false>(name,value,1)!=1 {terminate(42)}
            match read_volatile(value) {b'0'=>{},b'1'=>break,_=>terminate(42)}
        }
        if read_volatile(data)!=actor || read_volatile(zero)!=actor^0xff || read_volatile(stack)!=!actor {terminate(42)}
        terminate(0)
    }
}
