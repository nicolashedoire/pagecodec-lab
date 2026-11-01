#![no_std]
#![no_main]
#![recursion_limit = "256"]
mod mmu;
mod el0;
mod loader;
mod vfs;
extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use actor_core_sketch::{ActorId, ActorKind, Position, Registry, SpaceId};
use system_core_sketch::{Document, Origin};
use workspace_bridge_sketch::Bridge;
use dynamic_input_sketch::{Bounds, Router};
use serde_json::{json, Value};
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use virtio_drivers::{device::{blk::VirtIOBlk, gpu::VirtIOGpu, input::VirtIOInput}, transport::{mmio::{MmioTransport, VirtIOHeader}, DeviceType, Transport}, BufferDirection, Hal, PhysAddr};

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
    tbnz x0, #12, 3f
    tbnz x0, #25, 3f
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
// A failed write or confirmation can leave a newer snapshot on disk while the
// in-memory catalog still reflects the previous one. Block subsequent writes
// until a fresh boot has restored from the device instead of losing data.
static STORAGE_WRITE_BLOCKED: AtomicBool = AtomicBool::new(false);
#[cfg(feature="storage-fault-test")]
static FAIL_NEXT_POSTCOMMIT: AtomicBool = AtomicBool::new(false);

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

// Single-CPU QEMU, MMU enabled with an identity map and caches disabled.
// Full mapped-range checks are guest-specific, not suitable for real DMA.
struct VmHal;
unsafe impl Hal for VmHal {
    fn dma_alloc(pages: usize, _: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let size = pages.checked_mul(PAGE).expect("DMA overflow");
        let offset = DMA_NEXT.fetch_add(size, Ordering::Relaxed);
        assert!(size != 0 && offset <= DMA_SIZE && size <= DMA_SIZE - offset);
        // SAFETY: each allocation is page-aligned, non-overlapping and zeroed;
        // all pointers address reserved guest RAM for this one-shot test.
        let p = unsafe { (addr_of_mut!(DMA) as *mut u8).add(offset) };
        assert!(in_guest_ram(p as usize, size) && mmu::shareable_range(p as usize,size));
        (p as PhysAddr, NonNull::new(p).unwrap())
    }
    unsafe fn dma_dealloc(_: PhysAddr, _: NonNull<u8>, _: usize) -> i32 { 0 }
    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, size: usize) -> NonNull<u8> {
        assert!(size > 0 && paddr >= MMIO_BASE as u64 && paddr.checked_add(size as u64).is_some_and(|n| n <= (MMIO_BASE + MMIO_STRIDE * MMIO_SLOTS) as u64));
        assert!(mmu::identity_range(paddr as usize,size));
        NonNull::new(paddr as *mut u8).unwrap()
    }
    unsafe fn share(buffer: NonNull<[u8]>, _: BufferDirection) -> PhysAddr {
        let addr = buffer.as_ptr() as *mut u8 as usize;
        assert!(in_guest_ram(addr, buffer.len()) && mmu::shareable_range(addr,buffer.len()));
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
#[derive(Default)]
struct Interaction { human_focus:u8, ai_focus:u8, human_clicks:u32, ai_clicks:u32 }
fn focus_name(focus:u8)->&'static str {match focus {1=>"action",2=>"select",_=>"none"}}
fn button_hit(point:(u32,u32), ai:bool)->Option<u8> {
    let (x,y)=point;
    if !(430..465).contains(&y) { return None }
    let left=if ai {441} else {49};
    if (left..left+150).contains(&x) {Some(1)}
    else if (left+161..left+311).contains(&x) {Some(2)}
    else {None}
}
fn draw(buf: &mut [u8], human: (u32,u32), ai: (u32,u32), remote: (u32,u32), human_click: bool, state:&Interaction) {
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
    for (x,focus) in [(49,state.human_focus),(441,state.ai_focus)] {
        rounded(buf,x,430,150,35,if focus==1 {0x416d68} else {0x344d69});
        rounded(buf,x+161,430,150,35,if focus==2 {0x416d68} else {0x344d69});
        text(buf,x+14,442,b"ACTION",0xe5f3ff,1);
        text(buf,x+175,442,b"SELECT",0xe5f3ff,1);
    }
    text(buf,24,577,b"VIRTUAL MACHINE DEMO - NO OS CONTROL",0x8aa4c5,1);
    if human_click { rect(buf,human.0 as usize,human.1 as usize,24.min(PANEL_SPLIT as usize - human.0 as usize),24,0x2e7e72); }
    // Clamp only the rendered glyph at the framebuffer edges. Logical remote
    // coordinates remain unchanged; draw human last so it remains on top.
    let rx=(remote.0 as usize).min(WIDTH-15);
    let ry=(remote.1 as usize).min(HEIGHT-26);
    let label_x=if rx+16+72<WIDTH { rx+16 } else { rx.saturating_sub(72) };
    let label_y=ry.min(HEIGHT-20);
    rounded(buf,label_x,label_y,72,18,0x65481d);
    text(buf,label_x+5,label_y+4,b"REMOTE",0xffe4a6,1);
    pointer(buf,rx as u32,ry as u32,0xffc45f);
    pointer(buf,ai.0,ai.1,0x85aaff);
    pointer(buf,human.0,human.1,0x4ee8c8);
}

fn poll_uart() -> Option<u8> {
    // QEMU PL011 FR bit 4 indicates an empty receive FIFO.
    unsafe { if (0x0900_0018 as *const u32).read_volatile() & (1 << 4) != 0 { None } else { Some((0x0900_0000 as *const u32).read_volatile() as u8) } }
}
// JSON-RPC over the guest PL011 serial stream. This experimental server handles
// only an MCP 2025-11-25 subset. The disk is a synthetic QEMU device;
// actor labels and the serial endpoint are not authenticated.
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

// A synthetic in-memory document; labels are forgeable and confer no authority.
struct DocumentDemo {
    bridge: Bridge, base: Document, human: ActorId, assistant: ActorId,
    human_space: SpaceId, assistant_space: SpaceId,
}
impl DocumentDemo {
    fn new() -> Self {
        let mut base=Document::new();
        base.insert("title".into(),"base".into());
        base.insert("common".into(),"seed".into());
        Self::with_base(base)
    }
    fn with_base(base: Document) -> Self {
        let human=ActorId::new("human").unwrap();
        let assistant=ActorId::new("assistant").unwrap();
        let human_space=SpaceId::new("human-space").unwrap();
        let assistant_space=SpaceId::new("assistant-space").unwrap();
        let mut registry=Registry::new();
        registry.create_space(human_space.clone()).unwrap();
        registry.create_space(assistant_space.clone()).unwrap();
        registry.create_actor(human.clone(),ActorKind::Human).unwrap();
        registry.create_actor(assistant.clone(),ActorKind::Assistant).unwrap();
        registry.assign(&human,&human_space).unwrap();
        registry.assign(&assistant,&assistant_space).unwrap();
        let bridge=Bridge::new(registry,human.clone(),human_space.clone(),assistant.clone(),assistant_space.clone(),base.clone()).unwrap();
        Self { bridge,base,human,assistant,human_space,assistant_space }
    }
    fn snapshot(&self) -> Value {
        json!({"base":self.base,
               "human":self.bridge.view(&self.human,&self.human_space).unwrap(),
               "assistant":self.bridge.view(&self.assistant,&self.assistant_space).unwrap()})
    }
    fn from_snapshot(value: &Value) -> Option<Self> {
        let object=value.as_object()?;
        if object.len()!=3 {return None}
        let base=parse_document(object.get("base")?)?;
        let human=parse_document(object.get("human")?)?;
        let assistant=parse_document(object.get("assistant")?)?;
        let mut result=Self::with_base(base.clone());
        for (actor,space,copy) in [(&result.human,&result.human_space,&human),(&result.assistant,&result.assistant_space,&assistant)] {
            for key in base.keys() { if !copy.contains_key(key) { result.bridge.edit(actor,space,key,None).ok()?; } }
            for (key,value) in copy { if base.get(key)!=Some(value) { result.bridge.edit(actor,space,key,Some(value)).ok()?; } }
        }
        Some(result)
    }
    fn labels(&self, actor: &str, space: &str) -> Option<(&ActorId,&SpaceId)> {
        let a=match actor { "human"=>&self.human, "assistant"=>&self.assistant, _=>return None };
        let s=match space { "human-space"=>&self.human_space, "assistant-space"=>&self.assistant_space, _=>return None };
        Some((a,s))
    }
}
// Fixed bounded two-slot format. No authentication, transactions or crash guarantee.
const RECORD_BYTES:usize=4096;
fn parse_document(value:&Value)->Option<Document> {
    let object=value.as_object()?;
    if object.len()>16 {return None}
    let mut d=Document::new();
    for (key,value) in object {
        let text=value.as_str()?;
        if key.is_empty() || key.len()>32 || text.len()>64 {return None}
        d.insert(String::from(key),String::from(text));
    }
    Some(d)
}
// A slot is eight sectors: one metadata/commit sector and seven payload
// sectors. Checksums detect incidental corruption, never authenticate data.
const SLOT_SECTORS:[usize;2]=[4,12];
fn checksum_slot(data:&[u8;RECORD_BYTES],len:usize)->u32 {
    let mut h=2166136261u32;
    for &b in data[8..20].iter().chain(data[512..512+len].iter()) { h=(h^u32::from(b)).wrapping_mul(16777619); }
    h
}
fn snapshot_payload(doc:&DocumentDemo,files:&vfs::Files)->Value {
    json!({"format":"workspace-files-v1","document":doc.snapshot(),"files":files.snapshot()})
}
fn parse_payload(value:&Value)->Option<(DocumentDemo,vfs::Files)> {
    let object=value.as_object()?;
    if object.contains_key("format") {
        if object.len()!=3 || object.get("format")?.as_str()? != "workspace-files-v1" {return None}
        Some((DocumentDemo::from_snapshot(object.get("document")?)?,vfs::Files::from_snapshot(object.get("files")?)?))
    } else {
        // Only the exact former three-key document schema is accepted here.
        Some((DocumentDemo::from_snapshot(value)?,vfs::Files::default()))
    }
}
fn encode_snapshot(doc:&DocumentDemo,files:&vfs::Files,generation:u64)->Option<[u8;RECORD_BYTES]> {
    let v=snapshot_payload(doc,files); parse_payload(&v)?;
    let payload=serde_json::to_vec(&v).ok()?;
    if generation==0 || payload.is_empty() || payload.len()>RECORD_BYTES-512 {return None}
    let mut out=[0u8;RECORD_BYTES];out[..8].copy_from_slice(b"GSLOT001");
    out[8..16].copy_from_slice(&generation.to_le_bytes());
    out[16..20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    out[512..512+payload.len()].copy_from_slice(&payload);
    let sum=checksum_slot(&out,payload.len());
    out[20..24].copy_from_slice(&sum.to_le_bytes());
    // Byte 24 stays zero until a separate final metadata-sector write.
    Some(out)
}
fn decode_slot(data:&[u8;RECORD_BYTES])->Option<(u64,DocumentDemo,vfs::Files)> {
    if &data[..8]!=b"GSLOT001" || data[24]!=0xa5 || data[25..512].iter().any(|&b|b!=0) {return None}
    let generation=u64::from_le_bytes(data[8..16].try_into().ok()?);
    let len=u32::from_le_bytes(data[16..20].try_into().ok()?) as usize;
    if generation==0 || len==0 || len>RECORD_BYTES-512 {return None}
    let sum=u32::from_le_bytes(data[20..24].try_into().ok()?);
    if checksum_slot(data,len)!=sum || data[512+len..].iter().any(|&b|b!=0) {return None}
    let parsed:Value=serde_json::from_slice(&data[512..512+len]).ok()?;
    let (doc,files)=parse_payload(&parsed)?;
    Some((generation,doc,files))
}
// Surface degraded redundancy; a blank slot is distinct from a corrupt one.
// The checksum detects accidental corruption and is not authentication.
fn slot_state(data:&[u8;RECORD_BYTES])->&'static str {
    if data.iter().all(|&v|v==0) {"blank"}
    else if decode_slot(data).is_some() {"valid"}
    else {"invalid"}
}
// `Ok(None)` is allowed only for a completely blank pair. An invalid or
// interrupted first write is never silently mistaken for a fresh disk.
fn select_slots(a:&[u8;RECORD_BYTES],b:&[u8;RECORD_BYTES])->Result<Option<(usize,u64,DocumentDemo,vfs::Files)>, &'static str> {
    let x=decode_slot(a); let y=decode_slot(b);
    match (x,y) {
        (Some((ga,da,fa)),Some((gb,db,fb))) => {
            if ga==gb {Err("duplicate generations")} else if ga>gb {Ok(Some((0,ga,da,fa)))} else {Ok(Some((1,gb,db,fb)))}
        }
        (Some((g,d,f)),None)=>Ok(Some((0,g,d,f))),
        (None,Some((g,d,f)))=>Ok(Some((1,g,d,f))),
        (None,None) if a.iter().all(|&v|v==0) && b.iter().all(|&v|v==0)=>Ok(None),
        (None,None)=>Err("no valid committed snapshot"),
    }
}

fn read_pair<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>)->Result<([u8;RECORD_BYTES],[u8;RECORD_BYTES]),&'static str> {
    if disk.capacity()!=2048 {return Err("unexpected capacity")}
    let mut a=[0u8;RECORD_BYTES];let mut b=[0u8;RECORD_BYTES];
    disk.read_blocks(SLOT_SECTORS[0],&mut a).map_err(|_|"slot read failed")?;
    disk.read_blocks(SLOT_SECTORS[1],&mut b).map_err(|_|"slot read failed")?;
    Ok((a,b))
}
fn save_snapshot<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>,doc:&DocumentDemo,files:&vfs::Files)->Result<(usize,u64),&'static str> {
    if STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) { return Err("storage write blocked until restart"); }
    let (a,b)=read_pair(disk)?;
    let selected=select_slots(&a,&b)?;
    let (slot,generation)=match selected {None=>(0,1),Some((slot,g,_,_))=>(1-slot,g.checked_add(1).ok_or("generation exhausted")?)};
    let mut data=encode_snapshot(doc,files,generation).ok_or("snapshot invalid or capacity exceeded")?;
    // From this point a failed operation can leave either slot changed. This
    // one-CPU service serializes all writes; remain blocked on every error.
    STORAGE_WRITE_BLOCKED.store(true,Ordering::Relaxed);
    disk.write_blocks(SLOT_SECTORS[slot],&data).map_err(|_|"stage write failed")?;
    disk.flush().map_err(|_|"stage flush failed")?;
    data[24]=0xa5;
    disk.write_blocks(SLOT_SECTORS[slot],&data[..512]).map_err(|_|"commit write failed")?;
    disk.flush().map_err(|_|"commit flush failed")?;
    #[cfg(feature="storage-fault-test")]
    if FAIL_NEXT_POSTCOMMIT.swap(false,Ordering::Relaxed) { return Err("injected postcommit confirmation failure"); }
    // Do not report success before checking what the device returns. Readback
    // does not prove that a volatile cache persisted through power loss.
    let mut verified=[0u8;RECORD_BYTES];
    disk.read_blocks(SLOT_SECTORS[slot],&mut verified).map_err(|_|"readback failed")?;
    if verified!=data || !matches!(decode_slot(&verified),Some((g,_,_)) if g==generation) {
        return Err("readback mismatch");
    }
    STORAGE_WRITE_BLOCKED.store(false,Ordering::Relaxed);
    Ok((slot,generation))
}

// The sole EL1 service owns the catalog. Prepare changes privately and publish
// them only after the same validated two-slot write used by workspace saves.
fn put_file<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>,doc:&DocumentDemo,files:&mut vfs::Files,name:&str,data:&[u8])->Result<(usize,u64),&'static str> {
    let mut next=files.clone();
    next.put(name,data)?;
    let commit=save_snapshot(disk,doc,&next)?;
    *files=next;
    Ok(commit)
}
fn delete_file<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>,doc:&DocumentDemo,files:&mut vfs::Files,name:&str)->Result<(usize,u64),&'static str> {
    // Keep the boot image name reserved: deleting it and reseeding it on the
    // next boot would violate the persistent-delete contract.
    if matches!(name,"init.bin"|"fault.bin"|"badsvc.bin"|"next.bin") {return Err("reserved file")}
    let mut next=files.clone();
    next.remove(name)?;
    let commit=save_snapshot(disk,doc,&next)?;
    *files=next;
    Ok(commit)
}

fn tool_json(id: Value, data: Value) {
    send(&json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":format!("{}",data)}],"isError":false}}));
}
fn origin_name(origin: Origin) -> &'static str {
    match origin { Origin::Base=>"base", Origin::Human=>"human", Origin::Assistant=>"assistant", Origin::Both=>"both" }
}

// Actor/space labels are caller-supplied demonstration data, not credentials.
fn move_pointer(router: &mut Router, actor: &ActorId, space: &SpaceId, x: u32, y: u32) -> bool {
    router.move_pointer_bounded(actor, space, Position { x: x as i32, y: y as i32 }).is_ok()
}
fn cursor(router: &Router, actor: &ActorId) -> (u32,u32) {
    let point = router.actor(actor).and_then(|a|a.pointer()).unwrap();
    (point.x as u32, point.y as u32)
}

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let (private_a,private_b)=el0::private_stacks();
    mmu::init(addr_of_mut!(HEAP) as *mut u8 as usize,HEAP_SIZE,addr_of_mut!(DMA) as *mut u8 as usize,DMA_SIZE,private_a,private_b);
    el0::start()
}
#[no_mangle]
pub extern "C" fn graph_main() -> ! {
    let mut gpu = None;
    let mut input = None;
    let mut block = None;
    for slot in 0..MMIO_SLOTS {
        let ptr = NonNull::new((MMIO_BASE + slot * MMIO_STRIDE) as *mut VirtIOHeader).unwrap();
        // SAFETY: the specified QEMU virt machine maps each MMIO window; one CPU.
        let Ok(transport) = (unsafe { MmioTransport::new(ptr, MMIO_STRIDE) }) else { continue };
        match transport.device_type() {
            DeviceType::Block if block.is_none() => { block = VirtIOBlk::<VmHal, _>::new(transport).ok(); }
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
    let mut router = Router::new_bounded(64);
    let human=ActorId::new("human").unwrap(); let ai=ActorId::new("assistant").unwrap();
    let remote=ActorId::new("remote").unwrap();
    let hs=SpaceId::new("human-space").unwrap(); let ais=SpaceId::new("assistant-space").unwrap();
    let rs=SpaceId::new("remote-space").unwrap();
    // Bounds here use global display coordinates, even though the router itself
    // is coordinate-agnostic. The remote actor is logical, not another window.
    for (space, bounds) in [(&hs, Bounds {left:0, top:0, right:400, bottom:600}),
                            (&ais, Bounds {left:400, top:0, right:774, bottom:574}),
                            (&rs, Bounds {left:0, top:0, right:800, bottom:600})] {
        router.create_space(space.clone(), bounds).unwrap();
    }
    for (actor, kind, space) in [(&human,ActorKind::Human,&hs), (&ai,ActorKind::Assistant,&ais), (&remote,ActorKind::Remote,&rs)] {
        router.create_actor(actor.clone(),kind).unwrap(); router.assign(actor,space).unwrap();
    }
    if !move_pointer(&mut router, &human, &hs, 120, 180)
        || !move_pointer(&mut router, &ai, &ais, 560, 330)
        || !move_pointer(&mut router, &remote, &rs, 300, 510) { halt() }
    // The bounded journal overwrites its oldest entries without per-move allocation.
    let mut document = DocumentDemo::new();
    let mut files=vfs::Files::default();
    let restore_before=document.snapshot();
    let mut restore_status="unavailable";
    let mut restore_generation:Option<u64>=None;
    let mut restore_slots:Option<[&str;2]>=None;
    if let Some(ref mut disk)=block {
        match read_pair(disk).and_then(|(a,b)| {
            restore_slots=Some([slot_state(&a),slot_state(&b)]);
            select_slots(&a,&b)
        }) {
            Ok(Some((_,generation,restored,restored_files)))=>{document=restored;files=restored_files;restore_generation=Some(generation);restore_status="restored";}
            Ok(None)=>restore_status="blank",
            // Do not let a later transiently successful disk read overwrite
            // a valid snapshot with the fresh, unrestored in-memory state.
            Err(_)=>{restore_status="invalid";STORAGE_WRITE_BLOCKED.store(true,Ordering::Relaxed);}
        }
    } else {
        STORAGE_WRITE_BLOCKED.store(true,Ordering::Relaxed);
    }
    // Seed absent programs and migrate only the exact previously published
    // raw image. Unknown existing bytes are never silently replaced.
    let mut program_bootstrap=["unavailable";4];
    if matches!(restore_status,"blank"|"restored") {
        let migrate=files.get("init.bin").is_ok_and(|p|p==el0::legacy_program());
        for (which,name) in [(0,"init.bin"),(1,"fault.bin"),(2,"badsvc.bin"),(3,"next.bin")] {
            let absent=matches!(files.get(name),Err("file not found"));
            if absent || (which==0 && migrate) {
                program_bootstrap[which]=if let Some(program)=el0::approved_program(which) {
                    if files.put(name,&program).is_ok() {if absent {"seeded"}else{"migrated-raw"}} else {"capacity-error"}
                } else {"image-error"};
            } else {program_bootstrap[which]="present"}
        }
    }
    let mut storage_state=json!({"restore":restore_status,"restore_generation":restore_generation,"restore_slots":restore_slots,"program_bootstrap":program_bootstrap,"before":restore_before,"after":document.snapshot(),"save":null});
    let mut hclick = false;
    let mut interaction=Interaction::default();
    let mut pending_move: Option<(Value,u32,u32)> = None;
    let mut pending_remote: Option<(Value,u32,u32)> = None;
    let mut pending_ai_click: Option<Value> = None;
    // SAFETY: GPU owns this pinned buffer for its lifetime; one CPU and a draw
    // borrow ends before flush. QEMU-only HAL is not for production hardware.
    unsafe { draw(core::slice::from_raw_parts_mut(frame_ptr, FRAME_BYTES),
                  cursor(&router, &human), cursor(&router, &ai), cursor(&router, &remote), hclick, &interaction); }
    if gpu.flush().is_err() { halt() }
    el0::prepare_slices();
    // A complete 512-byte binary file requires 1,024 hexadecimal characters
    // plus the bounded JSON-RPC envelope; oversized lines remain rejected.
    let mut line = [0u8;1536];
    let mut used = 0usize;
    let mut overflow = false;
    let mut initialized = false;
    let mut ready = false;
    let mut messages = 0u32;
    // Avoid yielding between every byte of an incoming UART message. Even an
    // incomplete or absent message gets a bounded number of idle polls before
    // the service voluntarily schedules the EL0 slices.
    const SERVICE_POLLS: u32 = 20_000;
    let mut service_polls=SERVICE_POLLS;
    loop {
        if service_polls>=SERVICE_POLLS {
          // The fixed EL0 scripts assume a successful save and would halt on
          // failure. Keep them suspended when storage needs reconciliation so
          // the QEMU guest can still report its state and serve MCP reads.
          if !STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) {
            el0::run_slice([files.read_for_load("init.bin",512).ok(),files.read_for_load("fault.bin",512).ok(),files.read_for_load("badsvc.bin",512).ok(),files.read_for_load("next.bin",512).ok()]);
            // No EL0 task runs while this sole service owns the document.
            // Two entries at most can have been queued before this slice ends.
            for _ in 0..2 {
                let Some(request)=el0::pop_request() else {break};
                // Do not apply an IPC operation from a previous incarnation.
                // This check relies on the current single-CPU service model.
                if !el0::request_is_live(&request) {continue}
                if request.file_op!=0 {
                    // Requests were copied from a private EL0 page by SVC;
                    // authorize again at the owner boundary before disk access.
                    let key=request.key.get(..request.key_len).and_then(|x|core::str::from_utf8(x).ok());
                    let data=request.value.get(..request.value_len);
                    let mut reply=[0u8;32];
                    let mut reply_len=0usize;
                    let mut reply_too_large=false;
                    let mut reply_invalid_name=false;
                    let mut write_error=None;
                    let success=if request.actor!=0 {false} else if let (Some(key),Some(data))=(key,data) {
                        match request.file_op {
                            1=>{
                                // A restored, byte-identical committed file needs no rewrite.
                                if STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) {false}
                                else if files.get(key).is_ok_and(|old|old==data) {true}
                                else if let Some(ref mut disk)=block {
                                    match put_file(disk,&document,&mut files,key,data) {
                                        Ok(_)=>true,
                                        Err(error)=>{
                                            if request.id>=8 {
                                                write_error=Some(match error {
                                                    "invalid filename"=>el0::WriteError::InvalidName,
                                                    "too many files"|"catalog too large"|"file too large"|"snapshot invalid or capacity exceeded"=>el0::WriteError::NoSpace,
                                                    _=>el0::WriteError::Io,
                                                });
                                            }
                                            false
                                        }
                                    }
                                }
                                else {if request.id>=8 {write_error=Some(el0::WriteError::Io)} false}
                            }
                            2=>match files.get(key) {
                                Ok(bytes) if bytes.len()<=reply.len()=>{reply_len=bytes.len();reply[..reply_len].copy_from_slice(bytes);true},
                                Ok(_) if request.id>=8=>{reply_too_large=true;false},
                                Err("invalid filename") if request.id>=8=>{reply_invalid_name=true;false},
                                _=>false,
                            },
                            _=>false,
                        }
                    } else {false};
                    let completed=if let Some(error)=write_error {el0::complete_file_write_error(&request,error)} else if reply_too_large {el0::complete_file_oversize(&request)} else if reply_invalid_name {el0::complete_file_invalid_name(&request)} else {el0::complete_file(&request,success,&reply[..reply_len])};
                    if !completed {halt()}
                    continue;
                }
                let success=if request.save {
                    // Recheck authority at the owner boundary. IRQ/SVC never touch the disk.
                    if request.actor!=0 {false} else if let Some(ref mut disk)=block {
                        match save_snapshot(disk,&document,&files) {
                            Ok((slot,generation))=>{storage_state["save"]=json!({"slot":slot,"generation":generation,"readback_verified":true,"snapshot":document.snapshot()});true},
                            Err(_)=>{storage_state["save"]=json!({"error":"save failed"});false},
                        }
                    } else {false}
                } else if request.actor<2 {
                    let (actor,space)=if request.actor==0 {(&document.human,&document.human_space)} else {(&document.assistant,&document.assistant_space)};
                    match (request.key.get(..request.key_len).and_then(|x|core::str::from_utf8(x).ok()),
                           request.value.get(..request.value_len).and_then(|x|core::str::from_utf8(x).ok())) {
                        (Some(key),Some(value)) if !key.is_empty() && key.len()<=16 && value.len()<=32 =>
                            document.bridge.edit(actor,space,key,Some(value)).is_ok(),
                        _=>false,
                    }
                } else {false};
                if !el0::complete(&request,success) {halt()}
            }
          }
            service_polls=0;
        }
        service_polls+=1;
        let mut changed = false;
        while let Some(ev) = input.pop_pending_event() {
            service_polls=SERVICE_POLLS;
            match (ev.event_type,ev.code) {
                (3,0) => {
                    let x = (ev.value.min(32767) * (PANEL_SPLIT - 22) / 32767 + 16).min(390);
                    let (_, y) = cursor(&router, &human);
                    if !move_pointer(&mut router, &human, &hs, x, y) { halt() }
                    changed = true;
                }
                (3,1) => {
                    let y = ev.value.min(32767) * (HEIGHT as u32 - 100) / 32767 + 70;
                    let (x, _) = cursor(&router, &human);
                    if !move_pointer(&mut router, &human, &hs, x, y) { halt() }
                    changed = true;
                }
                (1,272) => {
                    let pressed=ev.value!=0;
                    if pressed && !hclick {
                        if let Some(target)=button_hit(cursor(&router,&human),false) {
                            interaction.human_focus=target;
                            interaction.human_clicks=interaction.human_clicks.saturating_add(1);
                        }
                    }
                    hclick=pressed; changed=true;
                }
                _ => {}
            }
        }
        while let Some(b) = poll_uart() {
            if b == b'\n' {
                service_polls=SERVICE_POLLS;
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
                                    Some("tools/list") => {
                                        #[allow(unused_mut)] // The fault tool exists only in test-feature builds.
                                        let mut response=json!({"jsonrpc":"2.0","id":id,"result":{"tools":[
                                        {"name":"move_ai","description":"Move AI pointer within its right-hand QEMU workspace.","inputSchema":{"type":"object","properties":{"x":{"type":"integer","minimum":400,"maximum":773},"y":{"type":"integer","minimum":0,"maximum":573}},"required":["x","y"],"additionalProperties":false}},
                                        {"name":"pointer_state","description":"Read the two pointer coordinates in this guest only.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"click_ai","description":"Activate the synthetic button under the AI pointer; no host click or authentication.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"interaction_state","description":"Read logical button focus and click counts in this guest.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"mmu_status","description":"Read the guest's verified identity-map status.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"el0_status","description":"Read verified pre-graphical EL0 timer trace.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"move_remote","description":"Move the visible pointer for a synthetic logical actor; no window or authenticated identity.","inputSchema":{"type":"object","properties":{"x":{"type":"integer","minimum":0,"maximum":799},"y":{"type":"integer","minimum":0,"maximum":599}},"required":["x","y"],"additionalProperties":false}},
                                        {"name":"remote_state","description":"Read the third synthetic actor's logical position.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"exercise_human_moves","description":"Run 1101 synthetic human moves in this VM; overwrites older bounded trace entries.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"storage_status","description":"Read synthetic startup restore and last SVC save status.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"file_get","description":"Read one synthetic flat file as hexadecimal bytes; no authentication.","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}},
                                        {"name":"file_put","description":"Save one bounded synthetic UTF-8 file with the current workspace snapshot; no authentication.","inputSchema":{"type":"object","properties":{"name":{"type":"string"},"data":{"type":"string"}},"required":["name","data"],"additionalProperties":false}},
                                        {"name":"file_put_hex","description":"Save bounded binary bytes encoded as lowercase hexadecimal; no authentication.","inputSchema":{"type":"object","properties":{"name":{"type":"string"},"hex":{"type":"string"}},"required":["name","hex"],"additionalProperties":false}},
                                        {"name":"file_list","description":"List synthetic file names and sizes; no authentication.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"file_delete","description":"Remove one synthetic file and save the workspace snapshot; no authentication.","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}},
                                        {"name":"ipc_status","description":"Read the bounded FIFO and task completion status.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"document_view","description":"Read a field from a synthetic document copy.","inputSchema":{"type":"object","properties":{"actor":{"type":"string","enum":["human","assistant"]},"space":{"type":"string","enum":["human-space","assistant-space"]},"key":{"type":"string","description":"Non-empty; at most 32 UTF-8 bytes."}},"required":["actor","space","key"],"additionalProperties":false}},
                                        {"name":"document_merge","description":"Merge synthetic copies and return fields, origins and conflicts.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                        {"name":"document_snapshot","description":"Read the synthetic base and both full copies.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
                                    ]}});
                                        #[cfg(feature="storage-fault-test")]
                                        response["result"]["tools"].as_array_mut().unwrap().push(json!({"name":"debug_fail_postcommit","description":"Test-only: fail the next write after commit flush.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}));
                                        send(&response);
                                    },
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
                                                    (Some(x),Some(y),true) if (400..=773).contains(&x) && y<=573 => {
                                                        if move_pointer(&mut router, &ai, &ais, x as u32, y as u32) {
                                                            pending_move=Some((id,x as u32,y as u32)); changed=true;
                                                        } else { tool_error(id,"pointer router rejected move"); }
                                                    }
                                                    _ => tool_error(id,"invalid or out-of-bounds coordinates"),
                                                }
                                            }
                                            Some("pointer_state") => {
                                                let args=params.and_then(|p| p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o| o.is_empty()) {
                                                    tool_error(id,"pointer_state requires an empty arguments object");
                                                } else {
                                                    let human_point = cursor(&router, &human);
                                                    let ai_point = cursor(&router, &ai);
                                                    let v = format!("human={},{}; ai={},{}",human_point.0,human_point.1,ai_point.0,ai_point.1);
                                                    send(&json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":v}],"isError":false}}));
                                                }
                                            }
                                            Some("storage_status") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"status requires empty arguments");}
                                                else {let mut status=storage_state.clone();status["write_blocked"]=json!(STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed));tool_json(id,status);}
                                            }
                                            #[cfg(feature="storage-fault-test")]
                                            Some("debug_fail_postcommit") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"debug requires empty arguments");}
                                                else if STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) {tool_error(id,"storage write blocked until restart");}
                                                else {FAIL_NEXT_POSTCOMMIT.store(true,Ordering::Relaxed);tool_json(id,json!({"armed":true}));}
                                            }
                                            Some("file_get") => {
                                                let args=params.and_then(|p|p.get("arguments")).and_then(Value::as_object);
                                                let name=args.and_then(|o|o.get("name")).and_then(Value::as_str);
                                                if !args.is_some_and(|o|o.len()==1) || name.is_none() {tool_error(id,"invalid file_get arguments");}
                                                else {match files.get(name.unwrap()) {
                                                    Err(error)=>tool_error(id,error),
                                                    Ok(_)=>tool_json(id,json!({"name":name,"hex":files.snapshot().get(name.unwrap()).cloned()})),
                                                }}
                                            }
                                            Some("file_list") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"file_list requires empty arguments");}
                                                else {tool_json(id,files.list());}
                                            }
                                            Some("file_put") => {
                                                let args=params.and_then(|p|p.get("arguments")).and_then(Value::as_object);
                                                let name=args.and_then(|o|o.get("name")).and_then(Value::as_str);
                                                let data=args.and_then(|o|o.get("data")).and_then(Value::as_str);
                                                if !args.is_some_and(|o|o.len()==2) || name.is_none() || data.is_none() {tool_error(id,"invalid file_put arguments");}
                                                else if let Some(ref mut disk)=block {
                                                    match put_file(disk,&document,&mut files,name.unwrap(),data.unwrap().as_bytes()) {
                                                        Ok((slot,generation))=>tool_json(id,json!({"slot":slot,"generation":generation,"readback_verified":true})),
                                                        Err(error)=>tool_error(id,error),
                                                    }
                                                } else {tool_error(id,"storage unavailable");}
                                            }
                                            Some("file_delete") => {
                                                let args=params.and_then(|p|p.get("arguments")).and_then(Value::as_object);
                                                let name=args.and_then(|o|o.get("name")).and_then(Value::as_str);
                                                if !args.is_some_and(|o|o.len()==1) || name.is_none() {tool_error(id,"invalid file_delete arguments");}
                                                else if let Some(ref mut disk)=block {
                                                    match delete_file(disk,&document,&mut files,name.unwrap()) {
                                                        Ok((slot,generation))=>tool_json(id,json!({"slot":slot,"generation":generation,"readback_verified":true})),
                                                        Err(error)=>tool_error(id,error),
                                                    }
                                                } else {tool_error(id,"storage unavailable");}
                                            }
                                            Some("file_put_hex") => {
                                                let args=params.and_then(|p|p.get("arguments")).and_then(Value::as_object);
                                                let name=args.and_then(|o|o.get("name")).and_then(Value::as_str);
                                                let hex=args.and_then(|o|o.get("hex")).and_then(Value::as_str);
                                                if !args.is_some_and(|o|o.len()==2) || name.is_none() || hex.is_none() {tool_error(id,"invalid file_put_hex arguments");}
                                                else {match vfs::decode_hex(hex.unwrap()) {
                                                    Err(error)=>tool_error(id,error),
                                                    Ok(bytes)=>if let Some(ref mut disk)=block {
                                                        match put_file(disk,&document,&mut files,name.unwrap(),&bytes) {
                                                            Ok((slot,generation))=>tool_json(id,json!({"slot":slot,"generation":generation,"readback_verified":true})),
                                                            Err(error)=>tool_error(id,error),
                                                        }
                                                    } else {tool_error(id,"storage unavailable");},
                                                }}
                                            }
                                            Some("ipc_status") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"status requires empty arguments");}
                                                else {tool_json(id,el0::ipc_status());}
                                            }
                                            Some("el0_status") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"status requires empty arguments");}
                                                else {tool_json(id,el0::status());}
                                            }
                                            Some("mmu_status") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"status requires empty arguments");}
                                                else {tool_json(id,json!({"enabled":true,"caches":false,"mapping":"identity","image":"normal-nc","virtio":"device-xn","range_checks":"full","kernel_el0":"samples-denied","user_code":"ro","user_stack":"private-alias-rw","address_spaces":2,"asids":[1,2],"gic":"device-xn"}));}
                                            }
                                            Some("interaction_state") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) { tool_error(id,"state requires empty arguments"); }
                                                else {tool_json(id,json!({"human":{"focus":focus_name(interaction.human_focus),"clicks":interaction.human_clicks,"pressed":hclick},"assistant":{"focus":focus_name(interaction.ai_focus),"clicks":interaction.ai_clicks}}));}
                                            }
                                            Some("click_ai") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) { tool_error(id,"click requires empty arguments"); }
                                                else if let Some(target)=button_hit(cursor(&router,&ai),true) {
                                                    interaction.ai_focus=target;
                                                    interaction.ai_clicks=interaction.ai_clicks.saturating_add(1);
                                                    pending_ai_click=Some(id);changed=true;
                                                } else { tool_error(id,"AI pointer not on a button"); }
                                            }
                                            Some("exercise_human_moves") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) { tool_error(id,"exercise requires empty arguments"); }
                                                else {
                                                    let before=HEAP_NEXT.load(Ordering::Relaxed);
                                                    let mut ok=true;
                                                    for i in 0..1100 { ok &= move_pointer(&mut router,&human,&hs,16+i%300,180); }
                                                    ok &= move_pointer(&mut router,&human,&hs,200,180);
                                                    let after=HEAP_NEXT.load(Ordering::Relaxed);
                                                    if !ok || before!=after || cursor(&router,&human)!=(200,180) || router.bounded_trace().len()!=64 || router.overwritten()==0 {
                                                        tool_error(id,"bounded movement test failed");
                                                    } else {
                                                        changed=true;
                                                        tool_json(id,json!({"moves":1101,"x":200,"y":180,"ring":router.bounded_trace().len(),"overwritten":router.overwritten(),"motion_allocations":0}));
                                                    }
                                                }
                                            }
                                            Some("move_remote") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                let x=args.and_then(|v|v.get("x")).and_then(Value::as_u64);
                                                let y=args.and_then(|v|v.get("y")).and_then(Value::as_u64);
                                                let exact=args.and_then(Value::as_object).is_some_and(|o|o.len()==2);
                                                match (x,y,exact) {
                                                    (Some(x),Some(y),true) if x<800 && y<600 => {
                                                        if move_pointer(&mut router,&remote,&rs,x as u32,y as u32) {
                                                            pending_remote=Some((id,x as u32,y as u32)); changed=true;
                                                        } else { tool_error(id,"remote move rejected"); }
                                                    }
                                                    _ => tool_error(id,"invalid or out-of-bounds remote coordinates"),
                                                }
                                            }
                                            Some("remote_state") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) { tool_error(id,"remote_state requires empty arguments"); }
                                                else { let (x,y)=cursor(&router,&remote); tool_json(id,json!({"actor":"remote","x":x,"y":y})); }
                                            }
                                            Some("document_view") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                let Some(obj)=args.and_then(Value::as_object) else {tool_error(id,"invalid arguments");used=0;overflow=false;break};
                                                let actor=obj.get("actor").and_then(Value::as_str);let space=obj.get("space").and_then(Value::as_str);let key=obj.get("key").and_then(Value::as_str);
                                                if obj.len()!=3 || !key.is_some_and(|k|!k.is_empty() && k.len()<=32) || actor.is_none() || space.is_none() {tool_error(id,"invalid arguments");}
                                                else if let Some((a,sp))=document.labels(actor.unwrap(),space.unwrap()) {
                                                    match document.bridge.view(a,sp) {
                                                        Ok(copy)=>tool_json(id,json!({"actor":actor,"space":space,"key":key,"value":copy.get(key.unwrap())})),
                                                        Err(_)=>tool_error(id,"actor or space rejected"),
                                                    }
                                                } else {tool_error(id,"unknown actor or space");}
                                            }
                                            Some("document_snapshot") => {
                                                let args=params.and_then(|p|p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o|o.is_empty()) {tool_error(id,"snapshot requires empty arguments");}
                                                else {tool_json(id,document.snapshot());}
                                            }
                                            Some("document_merge") => {
                                                let args=params.and_then(|p| p.get("arguments"));
                                                if !args.and_then(Value::as_object).is_some_and(|o| o.is_empty()) { tool_error(id,"merge requires empty arguments"); }
                                                else {
                                                    let merged=document.bridge.merge();
                                                    let fields:Vec<Value>=merged.fields.iter().map(|(key,field)|json!({"key":key,"value":field.value,"origin":origin_name(field.origin)})).collect();
                                                    let conflicts:Vec<Value>=merged.conflicts.iter().map(|c|json!({"key":c.key,"base":c.base,"human":c.human,"assistant":c.assistant})).collect();
                                                    tool_json(id,json!({"fields":fields,"conflicts":conflicts,"materializable":merged.conflicts.is_empty()}));
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
            unsafe { draw(core::slice::from_raw_parts_mut(frame_ptr, FRAME_BYTES),
                  cursor(&router, &human), cursor(&router, &ai), cursor(&router, &remote), hclick, &interaction); }
            if gpu.flush().is_err() {
                if let Some((id,_,_))=pending_move.take() { tool_error(id,"framebuffer flush failed"); }
                if let Some((id,_,_))=pending_remote.take() { tool_error(id,"framebuffer flush failed"); }
                if let Some(id)=pending_ai_click.take() {tool_error(id,"framebuffer flush failed");}
                halt()
            }
            if let Some(id)=pending_ai_click.take() {tool_json(id,json!({"focus":focus_name(interaction.ai_focus),"clicks":interaction.ai_clicks}));}
            if let Some((id,x,y))=pending_remote.take() { tool_json(id,json!({"actor":"remote","x":x,"y":y})); }
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
