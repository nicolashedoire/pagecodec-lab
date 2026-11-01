//! A freestanding Rust guest program for the integrated staging path.
//! One RX section, no allocator, no library/OS runtime and no global data.
#![no_std]
#![no_main]
use core::arch::asm;
use core::panic::PanicInfo;
#[panic_handler]
fn panic(_info:&PanicInfo<'_>)->! {loop {core::hint::spin_loop()}}

#[no_mangle]
#[link_section=".text"]
pub extern "C" fn _start(page:*mut u8, marker:u64)->! {
    unsafe {
        let cell=page.add(64).cast::<u64>();
        cell.write_volatile(marker);
        macro_rules! step {($n:expr)=>{{let value=cell.read_volatile();cell.write_volatile(value.rotate_left(5).bitxor($n).wrapping_add(0x1234));}}}
        use core::ops::BitXor;
        step!(1);
        step!(2);
        step!(3);
        step!(4);
        step!(5);
        step!(6);
        step!(7);
        step!(8);
        step!(9);
        step!(10);
        step!(11);
        step!(12);
        step!(13);
        step!(14);
        step!(15);
        step!(16);
        step!(17);
        step!(18);
        step!(19);
        step!(20);
        step!(21);
        step!(22);
        step!(23);
        step!(24);
        step!(25);
        step!(26);
        step!(27);
        step!(28);
        step!(29);
        step!(30);
        step!(31);
        step!(32);
        step!(33);
        step!(34);
        step!(35);
        step!(36);
        step!(37);
        step!(38);
        step!(39);
        step!(40);
        step!(41);
        step!(42);
        step!(43);
        step!(44);
        step!(45);
        step!(46);
        step!(47);
        step!(48);
        step!(49);
        step!(50);
        step!(51);
        step!(52);
        step!(53);
        step!(54);
        step!(55);
        step!(56);
        step!(57);
        step!(58);
        step!(59);
        step!(60);
        step!(61);
        step!(62);
        step!(63);
        step!(64);
        step!(65);
        step!(66);
        step!(67);
        step!(68);
        step!(69);
        step!(70);
        step!(71);
        step!(72);
        step!(73);
        step!(74);
        step!(75);
        step!(76);
        step!(77);
        step!(78);
        step!(79);
        step!(80);
        let mut status=if cell.read_volatile()==0x282b0828b2ffa84eu64 {0u64}else{42u64};
        let name=page.add(256);let data=page.add(320);
        name.add(0).write_volatile(b'a');name.add(1).write_volatile(b'-');
        name.add(2).write_volatile(b'r');name.add(3).write_volatile(b'u');
        name.add(4).write_volatile(b's');name.add(5).write_volatile(b't');
        data.write_volatile(b'R');
        if status==0 {
            loop {
                let mut result=name as u64;
                asm!("svc #0x141",inout("x0") result,in("x1") 6u64,in("x2") data as u64,in("x3") 1u64,options(nostack));
                if result==(-11i64) as u64 {continue}
                if result!=1 {status=42}
                break
            }
        }
        asm!("svc #0x123",in("x0") status,options(noreturn));
    }
}
