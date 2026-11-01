//! Two ELF processes exchange bounded messages over the guest's direct IPC.
#![no_std]
#![no_main]
use core::{arch::asm,panic::PanicInfo,ptr::{addr_of_mut,read_volatile,write_volatile}};
#[used] static mut INITIAL:u64=0x1122_3344_5566_7788;
#[used] static mut BUF:[u8;8]=[0;8];
#[panic_handler] fn panic(_: &PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}
unsafe fn stop(code:u64)->! {asm!("svc #0x123",in("x0") code,options(noreturn))}
unsafe fn peer()->(u64,u64,u64) {
    let mut slot=0u64;let incarnation:u64;let channel:u64;
    asm!("svc #0x160",inout("x0") slot,lateout("x1") incarnation,lateout("x2") channel,options(nostack));
    (slot,incarnation,channel)
}
unsafe fn await_peer(old:u64)->(u64,u64,u64) {
    loop {let (s,i,c)=peer();if s!=(-107i64) as u64 && i!=old {return (s,i,c)}}
}
unsafe fn send_once(slot:u64,inc:u64,channel:u64,buf:*const u8,len:u64)->u64 {
    let mut r=slot;asm!("svc #0x161",inout("x0") r,in("x1") inc,in("x2") buf as u64,in("x3") len,in("x4") channel,options(nostack));r
}
unsafe fn send(slot:u64,inc:u64,channel:u64,buf:*const u8,len:u64) {
    loop {let r=send_once(slot,inc,channel,buf,len);if r==0 {return}if r!=(-11i64) as u64 {stop(42)}}
}
unsafe fn recv_once(channel:u64,buf:*mut u8,cap:u64)->(u64,u64,u64) {
    let mut length=buf as u64;let mut source=cap;let mut incarnation=channel;
    asm!("svc #0x162",inout("x0") length,inout("x1") source,inout("x2") incarnation,options(nostack));
    (length,source,incarnation)
}
unsafe fn receive(channel:u64,buf:*mut u8,cap:u64)->(u64,u64,u64) {
    loop {let v=recv_once(channel,buf,cap);if v.0!=(-11i64) as u64 {return v}}
}
unsafe fn receive_pair(slot:u64,inc:u64,channel:u64,first:u8,second:u8,local:*mut u8,data:*mut u8) {
    write_volatile(local,0x5a);
    let small=receive(channel,local,1);
    if small.0!=(-34i64) as u64 || read_volatile(local)!=0x5a {stop(42)}
    let one=receive(channel,data,2);
    if one!=(2,slot,inc) || read_volatile(data)!=first || read_volatile(data.add(1))!=b'1' {stop(42)}
    let two=receive(channel,local,2);
    if two!=(2,slot,inc) || read_volatile(local)!=second || read_volatile(local.add(1))!=b'2' {stop(42)}
}
unsafe fn send_pair(slot:u64,inc:u64,channel:u64,letter:u8,local:*mut u8,data:*mut u8) {
    write_volatile(local,letter);write_volatile(local.add(1),b'1');send(slot,inc,channel,local,2);
    write_volatile(data,letter);write_volatile(data.add(1),b'2');send(slot,inc,channel,data,2);
}
#[no_mangle]
pub extern "C" fn _start(_page:*mut u8,actor:u64)->! {
    unsafe {
        if actor!=0xa1a1 && actor!=0xb2b2 {stop(42)}
        if read_volatile(addr_of_mut!(INITIAL))!=0x1122_3344_5566_7788 {stop(42)}
        let data=addr_of_mut!(BUF).cast::<u8>();
        if (0..8).any(|i|read_volatile(data.add(i))!=0) {stop(42)}
        let mut stack=[0xa5u8,0,0,0x5a];let local=stack.as_mut_ptr().add(1);
        if (local as usize)<0x50003000 || (local as usize)+2>0x50004000 {stop(42)}
        let (slot,inc,channel)=await_peer(0);
        if slot>1 || inc==0 {stop(42)}
        if send_once(slot,inc,channel,local,0)!=(-22i64) as u64 || send_once(slot,inc,channel,local,33)!=(-22i64) as u64 || send_once(slot,inc,channel,0x50003fffusize as *const u8,2)!=(-14i64) as u64 {stop(42)}
        if send_once(slot,inc+1,channel,local,1)!=(-116i64) as u64 {stop(42)}
        if actor==0xa1a1 {
            send_pair(slot,inc,channel,b'A',local,data);
            receive_pair(slot,inc,channel,b'B',b'B',local,data);
        } else {
            receive_pair(slot,inc,channel,b'A',b'A',local,data);
            send_pair(slot,inc,channel,b'B',local,data);
            let (new_slot,new_inc,new_channel)=await_peer(inc);
            if new_slot!=slot || new_inc==inc {stop(42)}
            if send_once(new_slot,inc,new_channel,local,1)!=(-116i64) as u64 {stop(42)}
            receive_pair(new_slot,new_inc,new_channel,b'A',b'A',local,data);
            send_pair(new_slot,new_inc,new_channel,b'B',local,data);
            // Keep the link alive until A has consumed both replies and exited.
            while peer().0!=(-107i64) as u64 {}
        }
        if read_volatile(stack.as_ptr())!=0xa5 || read_volatile(stack.as_ptr().add(3))!=0x5a {stop(42)}
        stop(0)
    }
}
