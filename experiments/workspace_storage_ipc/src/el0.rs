//! Controlled QEMU EL0 timer probe, run once before the graphical guest.
use alloc::vec::Vec;
use serde_json::{json,Value};
use core::{arch::{asm,global_asm},ptr::{addr_of,addr_of_mut,read_volatile,write_volatile},sync::atomic::Ordering};
const PAGE:usize=4096;
const GICD:usize=0x0800_0000;const GICC:usize=0x0801_0000;
#[repr(C,align(4096))] struct UserStack([u8;PAGE]);
#[repr(C,align(16))] #[derive(Clone,Copy)] struct Frame {regs:[u64;31],sp:u64,elr:u64,spsr:u64}
#[repr(C)] #[derive(Clone,Copy)] struct Event {actor:u64,count:u64,pc:u64,spsr:u64,sp:u64,next:u64,kernel_sp:u64}
const EMPTY:Frame=Frame{regs:[0;31],sp:0,elr:0,spsr:0};
#[link_section=".userstack"] static mut A_STACK:UserStack=UserStack([0;PAGE]);
#[link_section=".userstack"] static mut B_STACK:UserStack=UserStack([0;PAGE]);
static mut KERNEL_DATA:u64=0x0123_4567_89ab_cdef;
static mut A_FRAME:Frame=EMPTY;static mut B_FRAME:Frame=EMPTY;
#[no_mangle] static mut SAVED:[u64;2]=[0;2];
#[no_mangle] static mut CURRENT:u64=0;
#[no_mangle] static mut TASK_STATE:[u64;2]=[0;2];
#[no_mangle] static mut READY_QUEUE:[u64;2]=[0;2];
#[no_mangle] static mut READY_HEAD:u64=0;
#[no_mangle] static mut READY_LEN:u64=0;
#[no_mangle] static mut TICKS:u64=0;
#[no_mangle] static mut DELTA:u64=0;
#[no_mangle] static mut EVENTS:[Event;8]=[Event{actor:0,count:0,pc:0,spsr:0,sp:0,next:2,kernel_sp:0};8];
static mut PROBE_DONE:u64=0;
// The service yields only at an outer loop boundary; IRQ does not access
// its graphics, VirtIO, allocator or workspace state.
#[no_mangle] static mut SLICE_MODE:u64=0;
#[no_mangle] static mut SLICE_IRQS:u64=0;
#[no_mangle] static mut SLICE_TOTAL:u64=0;
#[no_mangle] static mut SLICE_COUNTS:[u64;2]=[0;2];
#[no_mangle] static mut USER_TOPS:[u64;2]=[0;2];
#[no_mangle] static mut SLICE_DELTA:u64=0;
// Single CPU, EL1 service executes only between slices; IRQs are masked on
// exception entry. The request queue and completions are fixed-size.
#[derive(Clone,Copy,PartialEq,Eq)]
pub struct Request {pub actor:usize,pub id:u64,pub save:bool,pub key:[u8;16],pub key_len:usize,pub value:[u8;32],pub value_len:usize}
#[derive(Clone,Copy)] struct Completion {actor:usize,id:u64,popped:bool,applied:bool,success:bool,delivered:bool,consumed:bool}
const EMPTY_COMPLETION:Completion=Completion{actor:0,id:0,popped:false,applied:false,success:false,delivered:false,consumed:false};
struct Ipc {queue:[Option<Request>;2],head:usize,len:usize,completions:[Completion;4],accepted:[u64;4],accepted_len:usize,applied:[u64;4],applied_len:usize,consumed:[u64;4],consumed_len:usize,full:bool,full_consumed:bool,range:[bool;2],spoof:[bool;2],save_denied:bool}
static mut IPC:Ipc=Ipc{queue:[None;2],head:0,len:0,completions:[EMPTY_COMPLETION;4],accepted:[0;4],accepted_len:0,applied:[0;4],applied_len:0,consumed:[0;4],consumed_len:0,full:false,full_consumed:false,range:[false;2],spoof:[false;2],save_denied:false};
fn in_page(p:u64,n:usize)->bool {n>0 && p>=super::mmu::ALIAS as u64 && p.checked_add(n as u64).is_some_and(|end|end<=(super::mmu::ALIAS+PAGE) as u64)}
unsafe fn read64(p:u64)->u64 {let v:u64;asm!("ldr {v}, [{p}]",p=in(reg)p,v=out(reg)v,options(nostack,preserves_flags));v}
unsafe fn write64(p:u64,v:u64){asm!("str {v}, [{p}]",p=in(reg)p,v=in(reg)v,options(nostack,preserves_flags));}
unsafe fn readbyte(p:u64)->u8 {let v:u32;asm!("ldrb {v:w}, [{p}]",p=in(reg)p,v=out(reg)v,options(nostack,preserves_flags));v as u8}
fn descriptor(actor:usize,index:usize,id:u64)->Option<Request> {
 if actor>1 || index>2 {return None}
 let h=(super::mmu::ALIAS+128+index*32) as u64;
 if !in_page(h,32) {return None}
 let (kp,kn,vp,vn)=unsafe{(read64(h),read64(h+8),read64(h+16),read64(h+24))};
 if kn==0 || kn>16 || vn>32 || !in_page(kp,kn as usize) || !in_page(vp,vn as usize) {return None}
 let mut req=Request{actor,id,save:false,key:[0;16],key_len:kn as usize,value:[0;32],value_len:vn as usize};
 for i in 0..req.key_len {req.key[i]=unsafe{readbyte(kp+i as u64)}}
 for i in 0..req.value_len {req.value[i]=unsafe{readbyte(vp+i as u64)}}
 Some(req)
}
fn submit(state:&mut Ipc,req:Request)->bool {
 let i=(req.id-1) as usize;
 if i>=4 || state.completions[i].id!=0 || state.len==2 || state.accepted_len>=4 {return false}
 let slot=(state.head+state.len)%2;state.queue[slot]=Some(req);state.len+=1;
 state.completions[i]=Completion{actor:req.actor,id:req.id,..EMPTY_COMPLETION};
 state.accepted[state.accepted_len]=req.id;state.accepted_len+=1;true
}
pub fn pop_request()->Option<Request> {
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 // Keep both requests in the queue until the guest has actually seen it full.
 if !s.full_consumed || s.len==0 {return None}
 let r=s.queue[s.head].take()?;s.head=(s.head+1)%2;s.len-=1;
 s.completions[(r.id-1) as usize].popped=true;Some(r)
}
pub fn complete(actor:usize,id:u64,success:bool)->bool {
 let s=unsafe{&mut *addr_of_mut!(IPC)};if id==0 || id>4{return false}
 let c=&mut s.completions[(id-1) as usize];
 if c.actor!=actor || c.id!=id || !c.popped || c.applied || s.applied_len>=4{return false}
 c.applied=true;c.success=success;s.applied[s.applied_len]=id;s.applied_len+=1;true
}
pub fn ipc_status()->Value {
 let s=unsafe{&*addr_of!(IPC)};
 let finished=s.consumed_len==4 && s.applied_len==4 && s.full && s.full_consumed && s.range==[true;2] && s.spoof==[true;2] && s.completions.iter().all(|c|c.success);
 json!({"finished":finished,"capacity":2,"queue_len":s.len,"accepted":&s.accepted[..s.accepted_len],"applied":&s.applied[..s.applied_len],"consumed":&s.consumed[..s.consumed_len],"full_rejected":s.full,"full_consumed":s.full_consumed,"range_rejected":s.range,"spoof_rejected":s.spoof,"save_denied":s.save_denied,"source":"synthetic private EL0 pages, kernel-seeded"})
}
#[no_mangle]
extern "C" fn ipc_dispatch(frame:*mut Frame,esr:u64) {
 let f=unsafe{&*frame};let actor=unsafe{read_volatile(addr_of!(CURRENT))} as usize;
 if actor>=2 || super::mmu::active_root()!=super::mmu::root(actor) || esr>>26!=0x15 || esr&(1<<25)==0 || f.spsr&0xf!=0 {super::write("FAIL SVC ID\n");halt()}
 let (lo,hi)=if actor==0 {(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64)} else {(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64)};
 if f.elr<lo || f.elr>=hi {
  super::write("FAIL SVC PC actor=");super::write_decimal(actor);
  super::write(" expected=");super::write_decimal(unsafe{read64((super::mmu::ALIAS+56) as u64)} as usize);
  super::write(" id=");super::write_decimal(unsafe{read64((super::mmu::ALIAS+80) as u64)} as usize);
  super::write(" result=");super::write_decimal(unsafe{read64((super::mmu::ALIAS+112) as u64)} as usize);
  super::write("\n");halt()
 }
 if unsafe{read_volatile(addr_of!(SLICE_MODE))}==0 {return}
 let s=unsafe{&mut *addr_of_mut!(IPC)};let op=esr&0xffff;
 match op {
  0x100 if actor==0 && s.completions[0].id==0 => {if let Some(r)=descriptor(actor,0,1){if !submit(s,r){halt()}}else{halt()}},
  0x102 if actor==0 && s.completions[0].id==1 && s.completions[1].id==0 => {if let Some(r)=descriptor(actor,1,2){if !submit(s,r){halt()}}else{halt()}},
  0x104 if actor==0 && s.completions[1].id==2 && !s.full => {
   let Some(r)=descriptor(actor,1,3) else {halt()};let before=s.queue;let head=s.head;let len=s.len;
   if len!=2 || submit(s,r) || s.queue!=before || s.head!=head || s.len!=len {halt()}
   s.full=true;unsafe{write64((super::mmu::ALIAS+72) as u64,3)};
  },
  0x106 if actor==0 && s.full && !s.full_consumed => {if unsafe{read64((super::mmu::ALIAS+64) as u64)}==3 {s.full_consumed=true;unsafe{write64((super::mmu::ALIAS+64) as u64,0);write64((super::mmu::ALIAS+72) as u64,0)}}},
  0x101 if actor==1 && s.completions[0].consumed && s.completions[1].consumed && s.completions[2].id==0 => {if let Some(r)=descriptor(actor,0,3){if !submit(s,r){halt()}}else{halt()}},
  0x100 if actor==1 => s.spoof[actor]=true,
  0x101 if actor==0 => s.spoof[actor]=true,
  0x1fe => {if descriptor(actor,2,1).is_none(){s.range[actor]=true}else{halt()}},
  0x110 if actor==1 => {s.save_denied=true;},
  0x110 if actor==0 && s.consumed_len==3 && s.completions[3].id==0 => {
   let r=Request{actor,id:4,save:true,key:[0;16],key_len:0,value:[0;32],value_len:0};
   if !submit(s,r) {super::write("FAIL SAVE ENQUEUE\n");halt()}
  },
  0x103 => {
    if !s.completions.iter().any(|c|c.actor==actor && c.delivered && !c.consumed) {
     if let Some(c)=s.completions.iter_mut().find(|c|c.id!=0 && c.actor==actor && c.applied && !c.delivered) {
      unsafe{write64((super::mmu::ALIAS+80) as u64,c.id);write64((super::mmu::ALIAS+112) as u64,if c.success {1}else{2})};c.delivered=true;
     }
    }
  },
  0x105 => {
   let observed=unsafe{read64((super::mmu::ALIAS+88) as u64)};let result=unsafe{read64((super::mmu::ALIAS+120) as u64)};
   if let Some(c)=s.completions.iter_mut().find(|c|c.id==observed && c.id!=0 && c.actor==actor && c.delivered && !c.consumed && result==if c.success {1}else{2}) {
    if s.consumed_len>=4 {halt()} c.consumed=true;s.consumed[s.consumed_len]=observed;s.consumed_len+=1;
    unsafe{write64((super::mmu::ALIAS+80) as u64,0);write64((super::mmu::ALIAS+88) as u64,0);write64((super::mmu::ALIAS+112) as u64,0);write64((super::mmu::ALIAS+120) as u64,0)};
   }
  },
  _=>{}
 }
}
// SP, x18, x19..x30; preserved EL1 call continuation.
#[no_mangle] static mut KERNEL_CONTEXT:[u64;14]=[0;14];
extern "C" {
 static __image_start:u8;static __image_end:u8;static __text_end:u8;
 static __usertext_start:u8;static __usertext_end:u8;static __userstack_start:u8;static __userstack_end:u8;
 static __bss_end:u8;static __stack_top:u8;static vector_table:u8;
 static task_a_loop:u8;static task_a_end:u8;static task_b_loop:u8;static task_b_end:u8;
 fn start_first(frame:*const Frame)->!;fn resume_graphics()->!;fn slice_probe(frame:*const Frame)->u64;
}
// Frame diagnostics are reached only after all 272 bytes have been saved.
// Output is synthetic guest context, emitted once on a fatal path.
#[no_mangle]
extern "C" fn irq_report(frame:*const Frame,family:u64)->! {
 let f=unsafe{read_volatile(frame)};let actor=unsafe{read_volatile(addr_of!(CURRENT))};
 super::write("FAIL IRQ family=");super::write_decimal(family as usize);
 super::write(" current=");super::write_decimal(actor as usize);
 super::write(" ttbr=");super::write_decimal(super::mmu::active_root() as usize);
 super::write(" elr=");super::write_decimal(f.elr as usize);
 super::write(" spsr=");super::write_decimal(f.spsr as usize);
 super::write(" user_sp=");super::write_decimal(f.sp as usize);
 for i in 0..31 {super::write(" x");super::write_decimal(i);super::write("=");super::write_decimal(f.regs[i] as usize);}
 super::write("\n");loop{unsafe{asm!("wfe",options(nomem,nostack))}}
}
// Used by vectors and post-frame paths where SP must not be read as a Frame.
#[no_mangle]
extern "C" fn unexpected_vector()->! {
 let mut elr:u64;let mut spsr:u64;let mut esr:u64;unsafe{asm!("mrs {},ELR_EL1",out(reg) elr);asm!("mrs {},SPSR_EL1",out(reg) spsr);asm!("mrs {},ESR_EL1",out(reg) esr);}
 super::write("FAIL IRQ no_frame elr=");super::write_decimal(elr as usize);super::write(" spsr=");super::write_decimal(spsr as usize);super::write(" esr=");super::write_decimal(esr as usize);super::write("\n");loop{unsafe{asm!("wfe",options(nomem,nostack))}}
}
fn halt()->! {super::write("FAIL EL0 HALT\n");loop{unsafe{asm!("wfe",options(nomem,nostack))}}}
fn check(ok:bool,msg:&str){if !ok{super::write("FAIL EL0 ");super::write(msg);super::write("\r\n");halt()}}
fn physical(value:usize)->usize {let out:usize;unsafe{asm!("mov {out}, {value}",out=out(reg) out,value=in(reg) value,options(nomem,nostack,preserves_flags));}out}
pub fn private_stacks()->(usize,usize) {(addr_of_mut!(A_STACK) as usize,addr_of_mut!(B_STACK) as usize)}
fn frame(actor:u64,sp:usize,pc:usize)->Frame {
 let mut f=EMPTY;for i in 0..31 {f.regs[i]=1000+actor*100+i as u64;}
 f.regs[0]=super::mmu::ALIAS as u64;f.regs[1]=if actor==0 {0xa1a1}else{0xb2b2};f.regs[2]=0;f.regs[3]=0;f.regs[4]=0;
 f.regs[19]=0;f.sp=sp as u64;f.elr=pc as u64;
 f.spsr=if actor==0 {0xa000_0340}else{0x5000_0340};f
}

pub fn start()->! {
 let start=physical(addr_of!(__image_start) as usize);let end=physical(addr_of!(__image_end) as usize);
 let text_end=physical(addr_of!(__text_end) as usize);let ut0=physical(addr_of!(__usertext_start) as usize);let ut1=physical(addr_of!(__usertext_end) as usize);
 let us0=physical(addr_of!(__userstack_start) as usize);let us1=physical(addr_of!(__userstack_end) as usize);
 let a=physical(addr_of_mut!(A_STACK) as usize);let b=physical(addr_of_mut!(B_STACK) as usize);
 let kb=physical(addr_of!(__bss_end) as usize);let kt=physical(addr_of!(__stack_top) as usize);
 let vector=physical(addr_of!(vector_table) as usize);
 let apc=physical(addr_of!(task_a_loop) as usize);let aend=physical(addr_of!(task_a_end) as usize);
 let bpc=physical(addr_of!(task_b_loop) as usize);let bend=physical(addr_of!(task_b_end) as usize);
 let mut sp:usize;let mut sctlr:u64;unsafe{asm!("mov {},sp",out(reg) sp);asm!("mrs {},SCTLR_EL1",out(reg) sctlr);}
 check(sctlr&1!=0 && sctlr&((1<<2)|(1<<12))==0 && sp>kb && sp<=kt && sp%16==0,"MMU or kernel stack");
 check(start<text_end && text_end<=ut0 && ut0<ut1 && us0<us1 && us1==end && kt<=us0,"section bounds");
 check(ut0%PAGE==0 && ut0.checked_add(PAGE)==Some(ut1) && us0.checked_add(2*PAGE)==Some(us1) && a%PAGE==0 && b%PAGE==0 && a>=us0 && b>=us0 && a+PAGE<=us1 && b+PAGE<=us1 && (a+PAGE<=b || b+PAGE<=a),"EL0 stack bounds");
 check(apc>=ut0 && apc<aend && aend<=ut1 && bpc>=ut0 && bpc<bend && bend<=ut1,"EL0 code bounds");
 check(vector%2048==0 && vector>=start && vector+2048<=text_end,"VBAR bounds");
 check(core::mem::size_of::<Frame>()==272 && core::mem::size_of::<Event>()==56,"frame layout");
 for p in [addr_of_mut!(A_STACK).cast::<u8>(),addr_of_mut!(B_STACK).cast::<u8>()] {for i in 0..PAGE {unsafe{write_volatile(p.add(i),0)}}}
 for (actor,p) in [addr_of_mut!(A_STACK).cast::<u8>(),addr_of_mut!(B_STACK).cast::<u8>()].into_iter().enumerate() {
  unsafe{write_volatile(p.add(56).cast::<u64>(),if actor==0 {1}else{3})};
  let (key,v1,v2):(&[u8],&[u8],&[u8])=if actor==0 {(b"theme",b"human-one",b"human-two")}else{(b"layout",b"assistant",b"assistant")};
  for (offset,bytes) in [(256,key),(288,v1),(320,v2)] {for (i,&v) in bytes.iter().enumerate(){unsafe{write_volatile(p.add(offset+i),v)}}}
  for index in 0..3 {let h=128+index*32;let fields=[if index==2 {(super::mmu::ALIAS+PAGE-1) as u64}else{(super::mmu::ALIAS+256) as u64},if index==2 {4}else{key.len() as u64},(super::mmu::ALIAS+if index==1 {320}else{288}) as u64,if index==1 {v2.len() as u64}else{v1.len() as u64}];for (i,v) in fields.into_iter().enumerate(){unsafe{write_volatile(p.add(h+i*8).cast::<u64>(),v)}}}
 }

 unsafe{asm!("msr VBAR_EL1, {}","isb",in(reg) vector);}
 let mut vbar:u64;let mut freq:u64;unsafe{asm!("mrs {},VBAR_EL1",out(reg) vbar);asm!("mrs {},CNTFRQ_EL0",out(reg) freq);}
 check(vbar==vector as u64,"VBAR readback");let delta=freq/100;check(delta>0 && delta<=i32::MAX as u64,"timer frequency");
 unsafe {
  addr_of_mut!(A_FRAME).write(frame(0,super::mmu::ALIAS+PAGE,addr_of!(task_a_loop) as usize));addr_of_mut!(B_FRAME).write(frame(1,super::mmu::ALIAS+PAGE,addr_of!(task_b_loop) as usize));
  addr_of_mut!(USER_TOPS).cast::<u64>().write((super::mmu::ALIAS+PAGE) as u64);addr_of_mut!(USER_TOPS).cast::<u64>().add(1).write((super::mmu::ALIAS+PAGE) as u64);
  addr_of_mut!(SAVED).cast::<u64>().write(addr_of!(A_FRAME) as u64);addr_of_mut!(SAVED).cast::<u64>().add(1).write(addr_of!(B_FRAME) as u64);
  addr_of_mut!(TASK_STATE).cast::<u64>().write(2);addr_of_mut!(TASK_STATE).cast::<u64>().add(1).write(1);
  addr_of_mut!(READY_QUEUE).cast::<u64>().write(1);addr_of_mut!(READY_HEAD).write(0);addr_of_mut!(READY_LEN).write(1);addr_of_mut!(DELTA).write(delta);
  write_volatile((GICD+0x000) as *mut u32,0);write_volatile((GICC+0x000) as *mut u32,0);
  write_volatile((GICD+0x180) as *mut u32,1<<27);write_volatile((GICD+0x280) as *mut u32,1<<27);
  write_volatile((GICD+0x400+27) as *mut u8,0x80);write_volatile((GICC+0x004) as *mut u32,0xff);
  write_volatile((GICD+0x100) as *mut u32,1<<27);write_volatile((GICD+0x000) as *mut u32,1);write_volatile((GICC+0x000) as *mut u32,1);
  asm!("dsb sy","isb");asm!("msr CNTV_TVAL_EL0, {}",in(reg) delta);asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);asm!("isb");
  start_first(addr_of!(A_FRAME));
 }
}

/// Explicit service-boundary yielding, never arbitrary EL1 preemption.
pub fn prepare_slices() {
 // Test-only diagnostic injection; never enabled in normal builds.
 #[cfg(feature="irq-diagnostic-test")]
 unsafe {for i in 0..2 {let p=read_volatile(addr_of!(SAVED).cast::<u64>().add(i)) as *mut Frame;(*p).regs[5]=42;}}
 let mut freq:u64;let mut daif:u64;unsafe{asm!("mrs {},CNTFRQ_EL0",out(reg) freq);asm!("mrs {},DAIF",out(reg) daif);}
 check(daif&(1<<7)!=0,"kernel IRQ not masked");let delta=freq/200;check(delta>0 && delta<=i32::MAX as u64,"slice timer frequency");
 unsafe {
  write_volatile(addr_of_mut!(SLICE_DELTA),delta);write_volatile(addr_of_mut!(SLICE_MODE),1);
  write_volatile((GICD+0x000) as *mut u32,0);write_volatile((GICC+0x000) as *mut u32,0);
  write_volatile((GICD+0x180) as *mut u32,1<<27);write_volatile((GICD+0x280) as *mut u32,1<<27);
  write_volatile((GICD+0x400+27) as *mut u8,0x80);write_volatile((GICC+0x004) as *mut u32,0xff);
  write_volatile((GICD+0x100) as *mut u32,1<<27);write_volatile((GICD+0x000) as *mut u32,1);write_volatile((GICC+0x000) as *mut u32,1);
  asm!("dsb sy","isb");
 }
}
pub fn run_slice() {
 let mut daif:u64;unsafe{asm!("mrs {},DAIF",out(reg) daif);}check(daif&(1<<7)!=0,"service IRQ unmasked");
 check(super::mmu::active_root()==super::mmu::root(0),"service root");
 let current=unsafe{read_volatile(addr_of!(CURRENT))};check(current<2,"slice actor");
 let frame=unsafe{read_volatile(addr_of!(SAVED).cast::<u64>().add(current as usize))} as *const Frame;
 let before=unsafe{read_volatile(addr_of!(SLICE_TOTAL))};let delta=unsafe{read_volatile(addr_of!(SLICE_DELTA))};check(delta!=0,"slice not initialized");
 let counts_before=[unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>())},unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>().add(1))}];
 let allocations_before=super::HEAP_NEXT.load(Ordering::Relaxed);
 let mut sentinel=[0u64;2];let p=sentinel.as_mut_ptr();let ptr=physical(p as usize);
 let bottom=physical(addr_of!(__bss_end) as usize);let top=physical(addr_of!(__stack_top) as usize);
 check(ptr>=bottom && ptr.checked_add(16).is_some_and(|v|v<=top),"service sentinel stack");
 unsafe{p.write_volatile(0x1122_3344_5566_7788);p.add(1).write_volatile(0x0102_0304_0506_0708);}
 unsafe {
  write_volatile(addr_of_mut!(SLICE_IRQS),0);asm!("msr CNTV_TVAL_EL0, {}",in(reg) delta);asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);asm!("isb");
  check(slice_probe(frame)==1,"kernel call context");
 }
 let after=unsafe{read_volatile(addr_of!(SLICE_TOTAL))};check(before.checked_add(2)==Some(after),"slice IRQ count");
 let counts_after=[unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>())},unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>().add(1))}];
 check(counts_after[0]>counts_before[0] && counts_after[1]>counts_before[1],"EL0 slice progress");
 check(super::HEAP_NEXT.load(Ordering::Relaxed)==allocations_before,"IRQ allocation");
 check(unsafe{read_volatile(p)}==0x1122_3344_5566_7788 && unsafe{read_volatile(p.add(1))}==0x0102_0304_0506_0708,"service stack sentinel");
 let mut timer:u64;unsafe{asm!("mrs {},CNTV_CTL_EL0",out(reg) timer);asm!("mrs {},DAIF",out(reg) daif);}
 check(timer&1==0 && daif&(1<<7)!=0 && super::mmu::active_root()==super::mmu::root(0),"slice timer, IRQ mask or service root");
 check(unsafe{read_volatile(addr_of!(A_STACK).cast::<u64>())}==0xa1a1 && unsafe{read_volatile(addr_of!(B_STACK).cast::<u64>())}==0xb2b2,"private task data");
}
pub fn status()->Value {
 let done=unsafe{read_volatile(addr_of!(PROBE_DONE))}==1;
 let events:Vec<Value>=(0..8).map(|i|{let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};json!({"task":e.actor,"count":e.count,"next":e.next})}).collect();
 let counts:Vec<u64>=(0..2).map(|i|unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>().add(i))}).collect();
 let slice_ticks=unsafe{read_volatile(addr_of!(SLICE_TOTAL))};
 let values=[unsafe{read_volatile(addr_of!(A_STACK).cast::<u64>())},unsafe{read_volatile(addr_of!(B_STACK).cast::<u64>())}];
 json!({"validated":done,"ticks":8,"el":"EL0","events":events,"timer":"disabled","interrupts":"masked","slice_ticks":slice_ticks,"slice_counts":counts,"mode":"service-boundary","private_values":values,"service_asid":super::mmu::active_root()>>48})
}

global_asm!(r#"
    .section .text.vectors, "ax"
    .balign 2048
    .global vector_table
vector_table:
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b lower_el_svc          // lower EL AArch64 sync
    .balign 128
    b lower_el_irq          // lower EL AArch64 IRQ
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128
    b irq_fatal
    .balign 128

    .section .text.irq, "ax"
lower_el_irq:
    // Exception arrived from EL0 on SP_EL1. Save every integer register,
    // user SP and exception return state before using any as scratch.
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, SP_EL0
    str x0, [sp, #248]
    mrs x0, ELR_EL1
    str x0, [sp, #256]
    mrs x0, SPSR_EL1
    str x0, [sp, #264]
    and x1, x0, #0xf
    cbnz x1, irq_entry_fail
    mov x1, #0x08010000
    ldr w0, [x1, #0x0c]
    and w2, w0, #0x3ff
    cmp w2, #27
    b.ne irq_entry_fail
    msr CNTV_CTL_EL0, xzr
    isb
    dsb sy
    str w0, [x1, #0x10]
    dsb sy
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    ldr x2, [x0]
    cmp x2, #1
    b.hi irq_entry_fail
    adrp x4, ROOTS
    add x4, x4, :lo12:ROOTS
    ldr x5, [x4, x2, lsl #3]
    mrs x6, TTBR0_EL1
    cmp x5, x6
    b.ne irq_entry_fail
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    ldr x3, [x3, x2, lsl #3]
    // Copy the kernel-stack exception frame to this actor's preallocated frame.
    mov x4, sp
    mov x5, #34
1:  ldr x6, [x4], #8
    str x6, [x3], #8
    subs x5, x5, #1
    b.ne 1b
    adrp x5, SLICE_MODE
    add x5, x5, :lo12:SLICE_MODE
    ldr x5, [x5]
    cbnz x5, slice_interrupt
    adrp x5, TICKS
    add x5, x5, :lo12:TICKS
    ldr x6, [x5]
    cmp x6, #8
    b.hs irq_entry_fail
    adrp x7, EVENTS
    add x7, x7, :lo12:EVENTS
    mov x8, #56
    madd x7, x6, x8, x7
    str x2, [x7, #0]
    ldr x8, [sp, #152]
    str x8, [x7, #8]
    ldr x8, [sp, #256]
    str x8, [x7, #16]
    ldr x8, [sp, #264]
    str x8, [x7, #24]
    ldr x8, [sp, #248]
    str x8, [x7, #32]
    mov x8, #2
    str x8, [x7, #40]
    add x8, sp, #272
    str x8, [x7, #48]
    add x6, x6, #1
    str x6, [x5]
    cmp x6, #8
    b.eq all_done
    // RUNNING -> READY, enqueue current ID, then dequeue the oldest ready ID.
    adrp x4, TASK_STATE
    add x4, x4, :lo12:TASK_STATE
    ldr x5, [x4, x2, lsl #3]
    cmp x5, #2
    b.ne irq_entry_fail
    mov x5, #1
    str x5, [x4, x2, lsl #3]
    adrp x4, READY_LEN
    add x4, x4, :lo12:READY_LEN
    ldr x5, [x4]
    cmp x5, #1
    b.hi irq_entry_fail
    adrp x6, READY_HEAD
    add x6, x6, :lo12:READY_HEAD
    ldr x6, [x6]
    cmp x6, #1
    b.hi irq_entry_fail
    add x6, x6, x5
    and x6, x6, #1
    adrp x7, READY_QUEUE
    add x7, x7, :lo12:READY_QUEUE
    str x2, [x7, x6, lsl #3]
    add x5, x5, #1
    str x5, [x4]
    bl select_ready
    cmp x0, #2
    b.hs irq_entry_fail
    adrp x9, TICKS
    add x9, x9, :lo12:TICKS
    ldr x9, [x9]
    sub x9, x9, #1
    adrp x10, EVENTS
    add x10, x10, :lo12:EVENTS
    mov x11, #56
    madd x10, x9, x11, x10
    str x0, [x10, #40]
    mov x2, x0
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    str x2, [x0]
    adrp x4, ROOTS
    add x4, x4, :lo12:ROOTS
    ldr x5, [x4, x2, lsl #3]
    dsb ishst
    msr TTBR0_EL1, x5
    isb
    tlbi vmalle1
    dsb ish
    isb
    mrs x6, TTBR0_EL1
    cmp x5, x6
    b.ne irq_entry_fail
    adrp x5, DELTA
    add x5, x5, :lo12:DELTA
    ldr x5, [x5]
    msr CNTV_TVAL_EL0, x5
    mov x5, #1
    msr CNTV_CTL_EL0, x5
    isb
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    ldr x0, [x3, x2, lsl #3]
    add sp, sp, #272       // release kernel IRQ frame before returning to EL0
    b restore_user_frame
slice_interrupt:
    // Validate this exact interrupted EL0 frame before selecting another task.
    // Register sentinels are task-specific; x19 is the running counter.
    mov x7, #0x50000000
    ldr x8, [sp, #0]
    cmp x8, x7
    b.ne irq_register_fail
    cbnz x2, slice_marker_b
    mov x7, #0xa1a1
    b slice_marker_check
slice_marker_b:
    mov x7, #0xb2b2
slice_marker_check:
    ldr x8, [sp, #8]
    cmp x8, x7
    b.ne irq_register_fail
    ldr x8, [sp, #16]
    cmp x8, #5
    b.ls slice_x2_ok
    cmp x8, x7
    b.ne irq_register_fail
slice_x2_ok:
    ldr x8, [sp, #24]
    cmp x8, #7
    b.hi irq_register_fail
    ldr x8, [sp, #32]
    cmp x8, #1
    b.ne irq_register_fail
    mov x7, #1000
    mov x8, #100
    madd x7, x2, x8, x7
    ldr x8, [sp, #40]
    add x9, x7, #5
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #48]
    add x9, x7, #6
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #56]
    add x9, x7, #7
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #64]
    add x9, x7, #8
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #72]
    add x9, x7, #9
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #80]
    add x9, x7, #10
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #88]
    add x9, x7, #11
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #96]
    add x9, x7, #12
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #104]
    add x9, x7, #13
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #112]
    add x9, x7, #14
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #120]
    add x9, x7, #15
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #128]
    add x9, x7, #16
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #136]
    add x9, x7, #17
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #144]
    add x9, x7, #18
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #160]
    add x9, x7, #20
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #168]
    add x9, x7, #21
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #176]
    add x9, x7, #22
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #184]
    add x9, x7, #23
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #192]
    add x9, x7, #24
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #200]
    add x9, x7, #25
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #208]
    add x9, x7, #26
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #216]
    add x9, x7, #27
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #224]
    add x9, x7, #28
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #232]
    add x9, x7, #29
    cmp x8, x9
    b.ne irq_register_fail
    ldr x8, [sp, #240]
    add x9, x7, #30
    cmp x8, x9
    b.ne irq_register_fail
    adrp x7, USER_TOPS
    add x7, x7, :lo12:USER_TOPS
    ldr x7, [x7, x2, lsl #3]
    ldr x8, [sp, #248]
    cmp x8, x7
    b.ne irq_register_fail
    cbnz x2, slice_check_b
    adrp x7, task_a_loop
    add x7, x7, :lo12:task_a_loop
    adrp x9, task_a_end
    add x9, x9, :lo12:task_a_end
    mov x10, #0xa0000000
    b slice_check_pc
slice_check_b:
    adrp x7, task_b_loop
    add x7, x7, :lo12:task_b_loop
    adrp x9, task_b_end
    add x9, x9, :lo12:task_b_end
    mov x10, #0x50000000
slice_check_pc:
    ldr x8, [sp, #256]
    cmp x8, x7
    b.lo irq_state_fail
    cmp x8, x9
    b.hs irq_state_fail
    ldr x8, [sp, #264]
    mov x7, #0xf0000000
    orr x7, x7, #0x3ff
    and x8, x8, x7
    add x10, x10, #0x340
    cmp x8, x10
    b.ne irq_state_fail
    adrp x5, SLICE_COUNTS
    add x5, x5, :lo12:SLICE_COUNTS
    ldr x6, [sp, #152]
    str x6, [x5, x2, lsl #3]
    adrp x5, SLICE_TOTAL
    add x5, x5, :lo12:SLICE_TOTAL
    ldr x6, [x5]
    add x6, x6, #1
    str x6, [x5]
    adrp x5, SLICE_IRQS
    add x5, x5, :lo12:SLICE_IRQS
    ldr x6, [x5]
    add x6, x6, #1
    str x6, [x5]
    cmp x6, #2
    b.hi irq_schedule_fail
    eor x2, x2, #1
    str x2, [x0]
    cmp x6, #2
    b.eq slice_return_kernel
    adrp x4, ROOTS
    add x4, x4, :lo12:ROOTS
    ldr x5, [x4, x2, lsl #3]
    dsb ishst
    msr TTBR0_EL1, x5
    isb
    tlbi vmalle1
    dsb ish
    isb
    mrs x6, TTBR0_EL1
    cmp x5, x6
    b.ne irq_schedule_fail
    adrp x5, SLICE_DELTA
    add x5, x5, :lo12:SLICE_DELTA
    ldr x5, [x5]
    msr CNTV_TVAL_EL0, x5
    mov x5, #1
    msr CNTV_CTL_EL0, x5
    isb
    adrp x3, SAVED
    add x3, x3, :lo12:SAVED
    ldr x0, [x3, x2, lsl #3]
    add sp, sp, #272
    b restore_user_frame
slice_return_kernel:
    // Service always resumes on root A; kernel/graphics mappings are common.
    adrp x4, ROOTS
    add x4, x4, :lo12:ROOTS
    ldr x5, [x4]
    dsb ishst
    msr TTBR0_EL1, x5
    isb
    tlbi vmalle1
    dsb ish
    isb
    mrs x6, TTBR0_EL1
    cmp x5, x6
    b.ne irq_schedule_fail
    add sp, sp, #272
    adrp x9, KERNEL_CONTEXT
    add x9, x9, :lo12:KERNEL_CONTEXT
    ldr x10, [x9, #0]
    mov x11, sp
    cmp x10, x11
    b.ne irq_fatal
    adrp x10, resume_kernel_slice
    add x10, x10, :lo12:resume_kernel_slice
    msr ELR_EL1, x10
    mov x10, #0x3c5
    msr SPSR_EL1, x10
    eret

all_done:
    // Timer is disabled; keep every interrupt class masked for validation.
    msr daifset, #0xf
    b el0_finish

select_ready:
    adrp x1, READY_LEN
    add x1, x1, :lo12:READY_LEN
    ldr x2, [x1]
    cbz x2, select_empty
    cmp x2, #2
    b.hi irq_schedule_fail
    adrp x3, READY_HEAD
    add x3, x3, :lo12:READY_HEAD
    ldr x4, [x3]
    cmp x4, #1
    b.hi irq_schedule_fail
    adrp x5, READY_QUEUE
    add x5, x5, :lo12:READY_QUEUE
    ldr x0, [x5, x4, lsl #3]
    cmp x0, #2
    b.hs irq_schedule_fail
    adrp x5, TASK_STATE
    add x5, x5, :lo12:TASK_STATE
    ldr x6, [x5, x0, lsl #3]
    cmp x6, #1
    b.ne irq_schedule_fail
    mov x6, #2
    str x6, [x5, x0, lsl #3]
    eor x4, x4, #1
    str x4, [x3]
    sub x2, x2, #1
    str x2, [x1]
    ret
select_empty:
    mov x0, #2
    ret
irq_entry_fail:
    mov x1, #1
    b irq_report_frame
irq_register_fail:
    mov x1, #2
    b irq_report_frame
irq_state_fail:
    mov x1, #3
    b irq_report_frame
irq_schedule_fail:
    mov x1, #4
irq_report_frame:
    msr daifset, #0xf
    mov x0, sp
    b irq_report
irq_fatal:
    msr daifset, #0xf
    b unexpected_vector

    .global start_slice
start_slice:
    adrp x9, KERNEL_CONTEXT
    add x9, x9, :lo12:KERNEL_CONTEXT
    mov x10, sp
    str x10, [x9, #0]
    str x18, [x9, #8]
    stp x19, x20, [x9, #16]
    stp x21, x22, [x9, #32]
    stp x23, x24, [x9, #48]
    stp x25, x26, [x9, #64]
    stp x27, x28, [x9, #80]
    stp x29, x30, [x9, #96]
    // Select the next EL0 task's root/ASID before exception return.
    adrp x9, CURRENT
    add x9, x9, :lo12:CURRENT
    ldr x10, [x9]
    cmp x10, #1
    b.hi irq_fatal
    adrp x9, ROOTS
    add x9, x9, :lo12:ROOTS
    ldr x10, [x9, x10, lsl #3]
    dsb ishst
    msr TTBR0_EL1, x10
    isb
    tlbi vmalle1
    dsb ish
    isb
    mrs x11, TTBR0_EL1
    cmp x10, x11
    b.ne irq_fatal
    b restore_user_frame
resume_kernel_slice:
    adrp x9, KERNEL_CONTEXT
    add x9, x9, :lo12:KERNEL_CONTEXT
    ldr x10, [x9, #0]
    mov sp, x10
    ldr x18, [x9, #8]
    ldp x19, x20, [x9, #16]
    ldp x21, x22, [x9, #32]
    ldp x23, x24, [x9, #48]
    ldp x25, x26, [x9, #64]
    ldp x27, x28, [x9, #80]
    ldp x29, x30, [x9, #96]
    ret

    .global slice_probe
slice_probe:
    sub sp, sp, #128
    stp x18, x19, [sp, #0]
    stp x20, x21, [sp, #16]
    stp x22, x23, [sp, #32]
    stp x24, x25, [sp, #48]
    stp x26, x27, [sp, #64]
    stp x28, x29, [sp, #80]
    str x30, [sp, #96]
    mov x1, sp
    str x1, [sp, #104]
    mov x18, #118
    mov x19, #119
    mov x20, #120
    mov x21, #121
    mov x22, #122
    mov x23, #123
    mov x24, #124
    mov x25, #125
    mov x26, #126
    mov x27, #127
    mov x28, #128
    mov x29, #129
    bl start_slice
    cmp x18, #118
    b.ne slice_probe_fail
    cmp x19, #119
    b.ne slice_probe_fail
    cmp x20, #120
    b.ne slice_probe_fail
    cmp x21, #121
    b.ne slice_probe_fail
    cmp x22, #122
    b.ne slice_probe_fail
    cmp x23, #123
    b.ne slice_probe_fail
    cmp x24, #124
    b.ne slice_probe_fail
    cmp x25, #125
    b.ne slice_probe_fail
    cmp x26, #126
    b.ne slice_probe_fail
    cmp x27, #127
    b.ne slice_probe_fail
    cmp x28, #128
    b.ne slice_probe_fail
    cmp x29, #129
    b.ne slice_probe_fail
    mov x0, sp
    ldr x1, [sp, #104]
    cmp x0, x1
    b.ne slice_probe_fail
    mov x0, #1
    b slice_probe_end
slice_probe_fail:
    mov x0, #0
slice_probe_end:
    ldp x18, x19, [sp, #0]
    ldp x20, x21, [sp, #16]
    ldp x22, x23, [sp, #32]
    ldp x24, x25, [sp, #48]
    ldp x26, x27, [sp, #64]
    ldp x28, x29, [sp, #80]
    ldr x30, [sp, #96]
    add sp, sp, #128
    ret

    .global start_first
start_first:
restore_user_frame:
    mov x30, x0
    ldr x1, [x30, #248]
    msr SP_EL0, x1
    ldr x1, [x30, #256]
    msr ELR_EL1, x1
    ldr x1, [x30, #264]
    msr SPSR_EL1, x1
    ldp x0, x1, [x30, #0]
    ldp x2, x3, [x30, #16]
    ldp x4, x5, [x30, #32]
    ldp x6, x7, [x30, #48]
    ldp x8, x9, [x30, #64]
    ldp x10, x11, [x30, #80]
    ldp x12, x13, [x30, #96]
    ldp x14, x15, [x30, #112]
    ldp x16, x17, [x30, #128]
    ldp x18, x19, [x30, #144]
    ldp x20, x21, [x30, #160]
    ldp x22, x23, [x30, #176]
    ldp x24, x25, [x30, #192]
    ldp x26, x27, [x30, #208]
    ldr x28, [x30, #224]
    ldr x29, [x30, #232]
    ldr x30, [x30, #240]
    eret

    .section .text.resume, "ax"
    .global resume_graphics
resume_graphics:
    ldr x0, =__stack_top
    mov sp, x0
    b graph_main

    .section .text.svc_handler, "ax"
lower_el_svc:
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, SP_EL0
    str x0, [sp, #248]
    mrs x0, ELR_EL1
    str x0, [sp, #256]
    mrs x0, SPSR_EL1
    str x0, [sp, #264]
    mrs x1, ESR_EL1
    mov x0, sp
    bl ipc_dispatch
    mov x0, sp
    add sp, sp, #272
    b svc_restore_frame
svc_restore_frame:
    mov x30, x0
    ldr x1, [x30, #248]
    msr SP_EL0, x1
    ldr x1, [x30, #256]
    msr ELR_EL1, x1
    ldr x1, [x30, #264]
    msr SPSR_EL1, x1
    ldp x0, x1, [x30, #0]
    ldp x2, x3, [x30, #16]
    ldp x4, x5, [x30, #32]
    ldp x6, x7, [x30, #48]
    ldp x8, x9, [x30, #64]
    ldp x10, x11, [x30, #80]
    ldp x12, x13, [x30, #96]
    ldp x14, x15, [x30, #112]
    ldp x16, x17, [x30, #128]
    ldp x18, x19, [x30, #144]
    ldp x20, x21, [x30, #160]
    ldp x22, x23, [x30, #176]
    ldp x24, x25, [x30, #192]
    ldp x26, x27, [x30, #208]
    ldr x28, [x30, #224]
    ldr x29, [x30, #232]
    ldr x30, [x30, #240]
    eret

    .section .usertext, "ax"
    .global task_a_loop
    .global task_a_end
task_a_loop:
    ldr x2, [x0]
    cbnz x4, 4f
    cbz x2, 3f
    eor x3, x2, x1
    cbnz x3, irq_fatal
3:  str x1, [x0]
    mov x4, #1
    b 5f
4:  eor x3, x2, x1
    cbnz x3, irq_fatal
5:  str x1, [x0]
    str x1, [x0, #96]
    str x0, [x0, #104]
    svc #0x1fe
    svc #0x100
    svc #0x102
    svc #0x104
    // EL0 compares the full-queue response to E_FULL=3 before echoing it.
    ldr x2, [x0, #72]
    cbz x2, 9f
    eor x3, x2, #3
    cbnz x3, el0_failure
    str x2, [x0, #64]
9:  ldr x2, [x0]
    mov x3, #0
    svc #0x106
    svc #0x101
    svc #0x110
    svc #0x103
    // Each task compares the delivered ID with its next expected ID and
    // the result with success=1 before echoing both to the kernel.
    ldr x2, [x0, #80]
    cbz x2, 10f
    ldr x3, [x0, #56]
    eor x3, x3, x2
    cbnz x3, el0_failure
    ldr x3, [x0, #112]
    eor x3, x3, #1
    cbnz x3, el0_failure
    str x2, [x0, #88]
    ldr x2, [x0, #112]
    str x2, [x0, #120]
    ldr x2, [x0, #56]
    add x2, x2, #1
    eor x3, x2, #3
    cbnz x3, 12f
    add x2, x2, #1
12: str x2, [x0, #56]
10: ldr x2, [x0]
    mov x3, #0
    svc #0x105
    svc #0x1ff
    add x19, x19, #1
    b task_a_loop
task_a_end:
    .global task_b_loop
    .global task_b_end
task_b_loop:
    ldr x2, [x0]
    cbnz x4, 7f
    cbz x2, 6f
    eor x3, x2, x1
    cbnz x3, irq_fatal
6:  str x1, [x0]
    mov x4, #1
    b 8f
7:  eor x3, x2, x1
    cbnz x3, irq_fatal
8:  str x1, [x0]
    str x1, [x0, #96]
    str x0, [x0, #104]
    svc #0x1fe
    svc #0x100
    svc #0x102
    svc #0x104
    // EL0 compares the full-queue response to E_FULL=3 before echoing it.
    ldr x2, [x0, #72]
    cbz x2, 9f
    eor x3, x2, #3
    cbnz x3, el0_failure
    str x2, [x0, #64]
9:  ldr x2, [x0]
    mov x3, #0
    svc #0x106
    svc #0x101
    svc #0x110
    svc #0x103
    // Each task compares the delivered ID with its next expected ID and
    // the result with success=1 before echoing both to the kernel.
    ldr x2, [x0, #80]
    cbz x2, 10f
    ldr x3, [x0, #56]
    eor x3, x3, x2
    cbnz x3, el0_failure
    ldr x3, [x0, #112]
    eor x3, x3, #1
    cbnz x3, el0_failure
    str x2, [x0, #88]
    ldr x2, [x0, #112]
    str x2, [x0, #120]
    ldr x2, [x0, #56]
    add x2, x2, #1
    str x2, [x0, #56]
10: ldr x2, [x0]
    mov x3, #0
    svc #0x105
    svc #0x1ff
    add x19, x19, #1
    b task_b_loop
task_b_end:
el0_failure:
    svc #0x7bad
    b el0_failure
"#);

#[no_mangle]
pub extern "C" fn el0_finish()->! {
    check(unsafe{read_volatile(addr_of!(TICKS))}==8,"tick count");
    let kb=physical(addr_of!(__bss_end) as usize);let kt=physical(addr_of!(__stack_top) as usize);
    let mut previous=[0u64;2];
    for i in 0..8 {
        let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};let actor=(i%2) as u64;
        let (lo,hi)=if actor==0 {(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64)} else {(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64)};
        let expected_spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(e.actor==actor && e.next==if i<7 {1-actor} else {2},"selected actor or next");
        check(e.count>previous[actor as usize],"task did not progress");previous[actor as usize]=e.count;
        check(e.pc>=lo && e.pc<hi && e.pc%4==0,"user PC");
        check(e.spsr&0xf000_03ff==expected_spsr && e.sp as usize==super::mmu::ALIAS+PAGE,"user SPSR or SP");
        check(e.kernel_sp%16==0 && e.kernel_sp as usize>kb && (e.kernel_sp as usize)<=kt && (e.kernel_sp as usize).checked_sub(272).is_some_and(|n|n>=kb),"kernel IRQ stack");

    }
    for actor in 0..2usize {
        let saved=unsafe{read_volatile(addr_of!(SAVED).cast::<u64>().add(actor))} as *const Frame;
        let expected=if actor==0 {addr_of!(A_FRAME)} else {addr_of!(B_FRAME)};
        check(saved==expected,"frame pointer");let f=unsafe{read_volatile(expected)};
        let spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(f.sp as usize==super::mmu::ALIAS+PAGE && f.spsr&0xf000_03ff==spsr,"final user frame");
        let marker=if actor==0 {0xa1a1} else {0xb2b2};
        check(f.regs[0]==super::mmu::ALIAS as u64 && f.regs[1]==marker && (f.regs[2]==marker || f.regs[2]==0) && f.regs[3]==0 && f.regs[4]==1,"private data registers");
        for i in 5..31 {if i!=19 {check(f.regs[i]==1000+actor as u64*100+i as u64,"GPR corruption")}}
        check(f.regs[19]==previous[actor],"counter mismatch");
    }
    let current=unsafe{read_volatile(addr_of!(CURRENT))} as usize;
    let head=unsafe{read_volatile(addr_of!(READY_HEAD))} as usize;
    let len=unsafe{read_volatile(addr_of!(READY_LEN))};
    check(current==1 && head<2 && len==1,"terminal ring shape");
    check(unsafe{read_volatile(addr_of!(READY_QUEUE).cast::<u64>().add(head))}==0
          && unsafe{read_volatile(addr_of!(TASK_STATE).cast::<u64>())}==1
          && unsafe{read_volatile(addr_of!(TASK_STATE).cast::<u64>().add(1))}==2,"terminal states");
    check(unsafe{read_volatile(addr_of!(KERNEL_DATA))}==0x0123_4567_89ab_cdef,"kernel data changed");
    check(unsafe{read_volatile(addr_of!(A_STACK).cast::<u64>())}==0xa1a1 && unsafe{read_volatile(addr_of!(B_STACK).cast::<u64>())}==0xb2b2,"private data markers");
    let mut ctl:u64;unsafe{asm!("mrs {}, CNTV_CTL_EL0",out(reg) ctl);}
    let active=unsafe{read_volatile((GICD+0x300) as *const u32)};
    check(ctl&1==0 && active&(1<<27)==0,"timer or GIC active");
    unsafe {
        write_volatile((GICD+0x180) as *mut u32,1<<27);
        write_volatile((GICC+0x000) as *mut u32,0);
        write_volatile((GICD+0x000) as *mut u32,0);
        asm!("dsb sy","isb");
        super::mmu::switch_root(super::mmu::root(0));
        write_volatile(addr_of_mut!(PROBE_DONE),1);
        resume_graphics();
    }
}
