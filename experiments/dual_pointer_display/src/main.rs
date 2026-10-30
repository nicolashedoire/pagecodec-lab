#![no_std]
#![no_main]
extern crate alloc;
use alloc::format;
use serde_json::{json, Value};
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};
use virtio_drivers::{device::{gpu::VirtIOGpu, input::VirtIOInput}, transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceType, Transport}, BufferDirection, Hal, PhysAddr};

global_asm!(r#"
    .section .text.boot, "ax"
    .global _start
_start:
    msr daifset, #0xf
    mrs x0, CurrentEL
    cmp x0, #4
    b.ne 3f
    mrs x0, SCTLR_EL1
    tbnz x0, #0, 3f
    tbnz x0, #2, 3f
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:
    cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:
    ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
3:
    wfe
    b 3b
"#);

const PAGE: usize = 4096;
const HEAP_SIZE: usize = 8 * 1024 * 1024;
const DMA_SIZE: usize = 8 * 1024 * 1024;
const MMIO_BASE: usize = 0x0a00_0000;
const MMIO_STRIDE: usize = 0x200;
const MMIO_SLOTS: usize = 32;
const RAM_BASE: usize = 0x4000_0000;
const RAM_END: usize = RAM_BASE + 128 * 1024 * 1024;

fn in_guest_ram(addr: usize, len: usize) -> bool {
    len > 0 && addr >= RAM_BASE && addr.checked_add(len).is_some_and(|end| end <= RAM_END)
}

#[repr(align(4096))]
struct PageAligned<const N: usize>([u8; N]);
static mut HEAP: PageAligned<HEAP_SIZE> = PageAligned([0; HEAP_SIZE]);
static mut DMA: PageAligned<DMA_SIZE> = PageAligned([0; DMA_SIZE]);
static HEAP_NEXT: AtomicUsize = AtomicUsize::new(0);
static DMA_NEXT: AtomicUsize = AtomicUsize::new(0);

struct Bump;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let base = addr_of_mut!(HEAP) as *mut u8 as usize;
        let mut current = HEAP_NEXT.load(Ordering::Relaxed);
        loop {
            let Some(start) = base.checked_add(current)
                .and_then(|n| n.checked_add(align - 1))
                .map(|n| (n & !(align - 1)) - base) else { return core::ptr::null_mut() };
            let Some(end) = start.checked_add(layout.size()) else { return core::ptr::null_mut() };
            if end > HEAP_SIZE { return core::ptr::null_mut(); }
            match HEAP_NEXT.compare_exchange_weak(current, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return (addr_of_mut!(HEAP) as *mut u8).add(start),
                Err(n) => current = n,
            }
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) { /* one-shot test, never reuse */ }
}
#[global_allocator]
static ALLOC: Bump = Bump;

// The VM is single-CPU with the MMU and caches disabled. Guest physical and
// virtual addresses are identical; this HAL is NOT suitable for real DMA.
struct VmHal;
unsafe impl Hal for VmHal {
    fn dma_alloc(pages: usize, _: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let size = pages.checked_mul(PAGE).expect("DMA overflow");
        let offset = DMA_NEXT.fetch_add(size, Ordering::Relaxed);
        assert!(size != 0 && offset <= DMA_SIZE && size <= DMA_SIZE - offset);
        // SAFETY: each allocation is page-aligned, non-overlapping and zeroed;
        // all pointers address reserved guest RAM for this one-shot test.
        let p = unsafe { (addr_of_mut!(DMA) as *mut u8).add(offset) };
        assert!(in_guest_ram(p as usize, size));
        (p as PhysAddr, NonNull::new(p).unwrap())
    }
    unsafe fn dma_dealloc(_: PhysAddr, _: NonNull<u8>, _: usize) -> i32 { 0 }
    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, size: usize) -> NonNull<u8> {
        assert!(size > 0 && paddr >= MMIO_BASE as u64 && paddr.checked_add(size as u64).is_some_and(|n| n <= (MMIO_BASE + MMIO_STRIDE * MMIO_SLOTS) as u64));
        NonNull::new(paddr as *mut u8).unwrap()
    }
    unsafe fn share(buffer: NonNull<[u8]>, _: BufferDirection) -> PhysAddr {
        let addr = buffer.as_ptr() as *mut u8 as usize;
        assert!(in_guest_ram(addr, buffer.len()));
        addr as PhysAddr
    }
    unsafe fn unshare(_: PhysAddr, _: NonNull<[u8]>, _: BufferDirection) {}
}

fn put(b: u8) {
    unsafe {
        while (0x0900_0018 as *const u32).read_volatile() & (1 << 5) != 0 { core::hint::spin_loop() }
        (0x0900_0000 as *mut u32).write_volatile(b as u32);
    }
}
fn write(s: &str) { for b in s.bytes() { put(b); } }
fn write_decimal(mut n: usize) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 { break; }
    }
    for &b in &digits[i..] { put(b); }
}
fn halt() -> ! { loop { unsafe { asm!("wfe", options(nomem, nostack)) } } }


const WIDTH: usize = 800;
const HEIGHT: usize = 600;
const FRAME_BYTES: usize = WIDTH * HEIGHT * 4;
const PANEL_SPLIT: u32 = 400;

fn pixel(buf: &mut [u8], x: usize, y: usize, rgb: u32) {
    if x >= WIDTH || y >= HEIGHT { return; }
    let i = (y * WIDTH + x) * 4;
    buf[i] = rgb as u8;
    buf[i + 1] = (rgb >> 8) as u8;
    buf[i + 2] = (rgb >> 16) as u8;
    buf[i + 3] = 255;
}
fn rect(buf: &mut [u8], x: usize, y: usize, w: usize, h: usize, color: u32) {
    let bottom = y.saturating_add(h).min(HEIGHT);
    let right = x.saturating_add(w).min(WIDTH);
    for yy in y.min(HEIGHT)..bottom { for xx in x.min(WIDTH)..right { pixel(buf, xx, yy, color); } }
}
fn rounded(buf: &mut [u8], x: usize, y: usize, w: usize, h: usize, color: u32) {
    // Eight-pixel rounded corners, clipped by rect at the framebuffer edges.
    for row in 0..h {
        let edge = row.min(h - 1 - row);
        let inset = match edge { 0 => 5, 1 => 3, 2 => 2, 3..=4 => 1, _ => 0 };
        if w > inset * 2 { rect(buf, x + inset, y + row, w - inset * 2, 1, color); }
    }
}
// Alpha cells generated from the bundled Inter font (SIL OFL 1.1).
const FONT_SMALL: &[u8] = include_bytes!("../assets/inter_small.bin");
const FONT_LARGE: &[u8] = include_bytes!("../assets/inter_large.bin");
fn text(buf: &mut [u8], x: usize, y: usize, s: &[u8], color: u32, scale: usize) {
    let (font,w,h) = if scale == 2 { (FONT_LARGE,20,22) } else { (FONT_SMALL,12,14) };
    let mut pen = x;
    for &ch in s {
        if !(32..=90).contains(&ch) { continue; }
        let base=(ch as usize-32)*(1+w*h);
        let advance=font[base] as usize;
        for row in 0..h { for col in 0..w {
            let alpha=font[base+1+row*w+col] as u32;
            let (px,py)=(pen+col,y+row);
            if alpha == 0 || px >= WIDTH || py >= HEIGHT { continue; }
            let i=(py*WIDTH+px)*4;
            for channel in 0..3 {
                let foreground=(color>>(channel*8))&255;
                buf[i+channel]=((foreground*alpha+buf[i+channel] as u32*(255-alpha)+127)/255) as u8;
            }
        } }
        pen+=advance;
    }
}
fn pointer(buf: &mut [u8], x: u32, y: u32, color: u32) {
    let (x,y)=(x as usize,y as usize);
    for yy in 0..22 { for xx in 0..15 { if xx <= yy/2 && yy < 20 { pixel(buf,x+xx,y+yy,0x09101d); } } }
    for yy in 1..18 { for xx in 1..13 { if xx < yy/2 { pixel(buf,x+xx,y+yy,color); } } }
    rect(buf,x+6,y+14,4,12,0x09101d); rect(buf,x+7,y+15,2,9,color);
}
fn draw(buf: &mut [u8], human: (u32,u32), ai: (u32,u32), human_click: bool) {
    rect(buf,0,0,WIDTH,HEIGHT,0x0b1020);
    rect(buf,0,0,WIDTH,54,0x101a2c);
    rect(buf,0,53,WIDTH,1,0x24354e);
    text(buf,24,18,b"DUAL WORKSPACE",0xe8f2ff,2);
    rounded(buf,675,14,101,27,0x233651);
    text(buf,690,24,b"QEMU GUEST",0xb2c8e3,1);
    rounded(buf,16,70,376,492,0x1a2a41);
    rounded(buf,408,70,376,492,0x1a2a41);
    rounded(buf,18,72,372,44,0x253b55);
    rounded(buf,410,72,372,44,0x253b55);
    rect(buf,31,86,10,10,0x48d6bb);
    rect(buf,423,86,10,10,0x7da8ff);
    text(buf,49,84,b"HUMAN WORKSPACE",0xe3efff,2);
    text(buf,441,84,b"AI WORKSPACE",0xe3efff,2);
    rounded(buf,31,136,346,135,0x24364f);
    rounded(buf,423,136,346,135,0x24364f);
    text(buf,49,155,b"YOUR POINTER",0xb2eadd,2);
    text(buf,441,155,b"AI POINTER",0xc6d8ff,2);
    text(buf,49,190,b"MOVE MOUSE IN VM",0x8aa4c5,1);
    text(buf,441,190,b"CONTROL VIA MCP",0x8aa4c5,1);
    rounded(buf,49,218,225,8,0x395775); rounded(buf,441,218,225,8,0x395775);
    rounded(buf,31,287,346,255,0x202f47); rounded(buf,423,287,346,255,0x202f47);
    text(buf,49,311,b"INDEPENDENT VIEW",0xb5c9df,1);
    text(buf,441,311,b"INDEPENDENT VIEW",0xb5c9df,1);
    rounded(buf,49,338,240,5,0x39516d); rounded(buf,49,359,285,5,0x39516d); rounded(buf,49,380,200,5,0x39516d);
    rounded(buf,441,338,240,5,0x39516d); rounded(buf,441,359,285,5,0x39516d); rounded(buf,441,380,200,5,0x39516d);
    text(buf,24,577,b"VIRTUAL MACHINE DEMO - NO OS CONTROL",0x8aa4c5,1);
    if human_click { rect(buf,human.0 as usize,human.1 as usize,24,24,0x2e7e72); }
    pointer(buf,human.0,human.1,0x4ee8c8);
    pointer(buf,ai.0,ai.1,0x85aaff);
}

fn poll_uart() -> Option<u8> {
    // QEMU PL011 FR bit 4 indicates an empty receive FIFO.
    unsafe { if (0x0900_0018 as *const u32).read_volatile() & (1 << 4) != 0 { None } else { Some((0x0900_0000 as *const u32).read_volatile() as u8) } }
}
// JSON-RPC over the guest PL011 serial stream. This experimental server handles
// only an MCP 2025-11-25 subset. It runs without network, grants, or persistence.
fn send(value: &Value) {
    if let Ok(s) = serde_json::to_string(value) { write(&s); write("\n"); }
}
fn rpc_error(id: Value, code: i32, msg: &str) {
    send(&json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":msg}}));
}
fn tool_error(id: Value, msg: &str) {
    send(&json!({"jsonrpc":"2.0", "id":id, "result":{"content":[{"type":"text","text":msg}],"isError":true}}));
}
fn valid_id(id: &Value) -> bool { id.is_string() || id.is_i64() || id.is_u64() }

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let mut gpu = None;
    let mut input = None;
    for slot in 0..MMIO_SLOTS {
        let ptr = NonNull::new((MMIO_BASE + slot * MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        // SAFETY: the specified QEMU virt machine maps each MMIO window; one CPU.
        let Ok(transport) = (unsafe { MmioTransport::new(ptr, MMIO_STRIDE) }) else { continue };
        match transport.device_type() {
            DeviceType::GPU if gpu.is_none() => { gpu = VirtIOGpu::<VmHal, _>::new(transport).ok(); }
            DeviceType::Input if input.is_none() => { input = VirtIOInput::<VmHal, _>::new(transport).ok(); }
            _ => {}
        }
    }
    let Some(mut gpu) = gpu else { halt() };
    let Some(mut input) = input else { halt() };
    let frame_ptr = match gpu.change_resolution(WIDTH as u32, HEIGHT as u32) {
        Ok(buf) if buf.len() >= FRAME_BYTES => buf.as_mut_ptr(),
        _ => halt()
    };
    let mut human = (120u32, 180u32);
    let mut ai = (560u32, 330u32);
    let mut hclick = false;
    let mut pending_move: Option<(Value,u32,u32)> = None;
    // SAFETY: GPU owns this pinned buffer for its lifetime; one CPU and a draw
    // borrow ends before flush. QEMU-only HAL is not for production hardware.
    unsafe { draw(core::slice::from_raw_parts_mut(frame_ptr, FRAME_BYTES), human, ai, hclick); }
    if gpu.flush().is_err() { halt() }
    let mut line = [0u8;512];
    let mut used = 0usize;
    let mut overflow = false;
    let mut initialized = false;
    let mut ready = false;
    let mut messages = 0u32;
    loop {
        let mut changed = false;
        while let Some(ev) = input.pop_pending_event() {
            match (ev.event_type,ev.code) {
                (3,0) => { human.0 = ev.value.min(32767) * (PANEL_SPLIT - 22) / 32767 + 16; changed = true; }
                (3,1) => { human.1 = ev.value.min(32767) * (HEIGHT as u32 - 100) / 32767 + 70; changed = true; }
                (1,272) => { hclick = ev.value != 0; changed = true; }
                _ => {}
            }
        }
        while let Some(b) = poll_uart() {
            if b == b'\n' {
                // Count even oversized requests. The bump heap is not reclaimed;
                // a bounded demo terminates instead of allocating indefinitely.
                if messages >= 64 {
                    write("{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32000,\"message\":\"demo request limit reached\"}}\n");
                    halt();
                }
                else {
                    messages += 1;
                    if overflow { rpc_error(Value::Null, -32700, "request too long"); }
                    else {
                    match serde_json::from_slice::<Value>(&line[..used]) {
                        Err(_) => rpc_error(Value::Null, -32700, "invalid JSON"),
                        Ok(msg) => {
                            let id = msg.get("id").cloned().unwrap_or(Value::Null);
                            let method = msg.get("method").and_then(Value::as_str);
                            if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                                rpc_error(Value::Null, -32600, "invalid request");
                            } else if method == Some("notifications/initialized") && initialized && id.is_null() {
                                ready = true;
                            } else if !valid_id(&id) {
                                if msg.get("id").is_some() { rpc_error(Value::Null,-32600,"invalid request id"); }
                                // Unknown notifications are ignored.
                            } else if method == Some("initialize") {
                                initialized = true;
                                ready = false;
                                send(&json!({"jsonrpc":"2.0","id":id,"result":{
                                    "protocolVersion":"2025-11-25",
                                    "capabilities":{"tools":{}},
                                    "serverInfo":{"name":"dual-pointer-vm","version":"0.1.0"}
                                }}));
                            } else if !ready {
                                rpc_error(id,-32002,"not initialized");
                            } else {
                                match method {
                                    Some("ping") => send(&json!({"jsonrpc":"2.0","id":id,"result":{}})),
                                    Some("tools/list") => send(&json!({"jsonrpc":"2.0","id":id,"result":{"tools":[
                                        {"name":"move_ai","description":"Move AI pointer within its right-hand QEMU workspace.","inputSchema":{"type":"object","properties":{"x":{"type":"integer","minimum":400,"maximum":773},"y":{"type":"integer","minimum":0,"maximum":573}},"required":["x","y"],"additionalProperties":false}},
                                        {"name":"pointer_state","description":"Read the two pointer coordinates in this guest only.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}
                                    ]}})),
                                    Some("tools/call") => {
                                        let params = msg.get("params");
                                        let name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
                                        match name {
                                            Some("move_ai") => {
                                                let args=params.and_then(|p| p.get("arguments"));
                                                let x=args.and_then(|v| v.get("x")).and_then(Value::as_u64);
                                                let y=args.and_then(|v| v.get("y")).and_then(Value::as_u64);
                                                let exactly_two=args.and_then(Value::as_object).is_some_and(|o| o.len()==2);
                                                match (x,y,exactly_two) {
                                                    (Some(x),Some(y),true) if (400..=773).contains(&x) && y<=573 => { ai=(x as u32,y as u32); pending_move=Some((id,ai.0,ai.1)); changed=true; }
                                                    _ => tool_error(id,"invalid or out-of-bounds coordinates"),
                                                }
                                            }
                                            Some("pointer_state") => {
                                                let args=params.and_then(|p| p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o| o.is_empty()) {
                                                    tool_error(id,"pointer_state requires an empty arguments object");
                                                } else {
                                                    let v = format!("human={},{}; ai={},{}",human.0,human.1,ai.0,ai.1);
                                                    send(&json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":v}],"isError":false}}));
                                                }
                                            }
                                            _ => rpc_error(id,-32602,"unknown tool"),
                                        }
                                    }
                                    _ => rpc_error(id,-32601,"method not found"),
                                }
                            }
                        }
                    }
                    }
                }
                used=0; overflow=false;
                // Complete one request at a time; reply to move after the GPU flush.
                break;
            } else if used < line.len() { line[used]=b; used+=1; } else { overflow=true; }
        }
        if changed {
            unsafe { draw(core::slice::from_raw_parts_mut(frame_ptr, FRAME_BYTES), human, ai, hclick); }
            if gpu.flush().is_err() { if let Some((id,_,_))=pending_move.take() { tool_error(id,"framebuffer flush failed"); } halt() }
            if let Some((id,x,y))=pending_move.take() {
                let value = format!("moved AI pointer to {},{}",x,y);
                send(&json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":value}],"isError":false}}));
            }
        }
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { halt() }
