//! Controlled QEMU EL0 timer probe, run once before the graphical guest.
use alloc::vec::Vec;
use serde_json::{json,Value};
use core::{arch::{asm,global_asm},ptr::{addr_of,addr_of_mut,read_volatile,write_volatile}};
const PAGE:usize=4096;
const GICD:usize=0x0800_0000;const GICC:usize=0x0801_0000;
#[repr(C,align(4096))] struct UserStack([u8;PAGE]);
#[repr(C,align(16))] #[derive(Clone,Copy)] struct Frame {regs:[u64;31],sp:u64,elr:u64,spsr:u64}
#[repr(C)] #[derive(Clone,Copy)] struct Event {actor:u64,count:u64,pc:u64,spsr:u64,sp:u64,next:u64,kernel_sp:u64}
const EMPTY:Frame=Frame{regs:[0;31],sp:0,elr:0,spsr:0};
// Fixed boot-reserved frame pool, not a general physical-memory allocator.
// Frames are exclusively assigned to task addresses and zeroed before mapping.
#[link_section=".userstack"] static mut USER_FRAMES:[UserStack;6]=[const {UserStack([0;PAGE])};6];
static mut FRAME_BITMAP:u8=0;
static mut TASK_FRAMES:[usize;2]=[0;2];
static mut CROSS_READ_DENIED:[bool;2]=[false;2];
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
#[no_mangle] static mut SLICE_DISPATCHES:u64=0;
#[no_mangle] static mut SLICE_EXITS:u64=0;
static mut FRAME_SCRUBBED:[bool;2]=[false;2];
static mut ALIAS_REMOVED:[bool;2]=[false;2];
static mut EXIT_MARKERS:[u64;2]=[0;2];
static mut FIRST_FRAME_A:usize=0;
static mut SPAWN_ADMITTED:bool=false;
static mut IMAGE_COUNT:u8=0;
// Pausing admits no new historical ELF; an already running one is allowed to
// finish. This flag never changes the recorded fixture progress.
static mut FIXTURE_PAUSE_REQUESTED:bool=false;
static mut INITIAL_TASKS_RECLAIMED:bool=false;
static mut IMAGE1_EXITED:bool=false;
static mut IMAGE2_EXITED:bool=false;
static mut IMAGE1_RECLAIMED:bool=false;
static mut IMAGE2_RECLAIMED:bool=false;
static mut IMAGE3_RECLAIMED:bool=false;
static mut IMAGE4_RECLAIMED:bool=false;
// 0 pending, 1 success, 2 synchronous fault, 3 unknown SVC,
// 4 reported file error, 5 execution budget exceeded, 6 nonzero exit.
static mut IMAGE_OUTCOMES:[u8;4]=[0;4];
// A generic exit records the caller's full x0 value before releasing its frame.
static mut IMAGE_EXIT_CODES:[Option<u64>;4]=[None;4];
static mut IMAGE_IRQ_X19_MAX_SEEN:[bool;4]=[false;4];
static mut PENDING_CANCELLED:u64=0;
static mut COMPLETIONS_REVOKED:[bool;4]=[false;4];
static mut STALE_RESPONSE_BLOCKED:bool=false;
// A slot identifies an address space, not a task lifetime. Kernel-assigned
// nonzero generations distinguish later admissions into the same slot.
static mut TASK_INCARNATIONS:[u64;2]=[1,2];
static mut NEXT_INCARNATION:u64=3;
static mut IMAGE_INCARNATIONS:[u64;4]=[0;4];
static mut STALE_TOKEN:Option<CompletionToken>=None;
static mut STALE_GENERATION_REJECTED:bool=false;
static mut FILE_EL0_VERIFIED:bool=false;
#[no_mangle] static mut SPAWN_ACTIVE:bool=false;
static mut SPAWN_EXECUTED:bool=false;
static mut SPAWN_EXITED:bool=false;
static mut SPAWN_FAILED:bool=false;
static mut SPAWN_REUSED:bool=false;
#[no_mangle] static mut PAIR_ACTIVE:bool=false;
// 0 never started, 1 single-slot mode, 2 pair mode, 3 independent slots.
static mut PAIR_MODE:u8=0;
static mut PAIR_STARTED:bool=false;
static mut PAIR_RUN_ID:u64=0;
// Tokens alone are retained for a delayed-reply rejection check; never payloads.
static mut PAIR_LAST_TOKENS:[Option<CompletionToken>;2]=[None;2];
static mut PAIR_STALE_TOKENS:[Option<CompletionToken>;2]=[None;2];
static mut PAIR_STALE_REJECTED:[bool;2]=[false;2];
static mut PAIR_OLD_INCARNATION_REJECTED:[bool;2]=[false;2];
static mut PAIR_DONE:bool=false;
static mut PAIR_INCARNATIONS:[u64;2]=[0;2];
static mut PAIR_CODE_FRAMES:[usize;2]=[0;2];
static mut PAIR_STACK_FRAMES:[usize;2]=[0;2];
static mut PAIR_STACK_SCRUBBED:[bool;2]=[false;2];
static mut PAIR_RW_INIT_VERIFIED:[bool;2]=[false;2];
static mut PAIR_TWO_SEGMENT:[bool;2]=[false;2];
static mut PAIR_CODE_LEN:[u64;2]=[0;2];
static mut PAIR_IRQS:[u64;2]=[0;2];
static mut PAIR_BARRIER_SEEN:[bool;2]=[false;2];
static mut PAIR_BARRIER_OPEN:bool=false;
// 0 running, 1 explicit exit, 2 fault or unsupported SVC, 3 IRQ budget,
// 4 stopped by the EL1 service (not an EL0 exit).
static mut PAIR_OUTCOMES:[u8;2]=[0;2];
static mut PAIR_LIVE_MARKERS:[u64;2]=[0;2];
static mut PAIR_IO_SUBMITTED:[u64;2]=[0;2];
static mut PAIR_IO_COMPLETED:[u64;2]=[0;2];
static mut PAIR_IO_CANCELLED:[u64;2]=[0;2];
static mut PAIR_EXIT_CODES:[Option<u64>;2]=[None;2];
static mut PAIR_MARKERS:[u64;2]=[0;2];
static mut PAIR_DATA_SCRUBBED:[bool;2]=[false;2];
static mut PAIR_CODE_SCRUBBED:[bool;2]=[false;2];
static mut PAIR_MAPPINGS_VERIFIED:bool=false;
static mut SLOT_MAPPINGS_VERIFIED:[bool;2]=[false;2];
static mut CODE_FRAME:usize=0;
static mut CODE_ALIAS_REMOVED:bool=false;
static mut CODE_FRAME_SCRUBBED:bool=false;
static mut LOADER_REJECTED:bool=false;
static mut IMAGE_LOADED:bool=false;
#[no_mangle] static mut SPAWN_CODE_LEN:u64=0;
#[no_mangle] static mut SPAWN_TICKS:u64=0;
// Diagnostic for the synthetic test: it does not affect scheduling.
#[no_mangle] static mut SPAWN_IRQ_X19_MAX_SEEN:bool=false;
#[no_mangle] static mut SLICE_COUNTS:[u64;2]=[0;2];
#[no_mangle] static mut USER_TOPS:[u64;2]=[0;2];
#[no_mangle] static mut SLICE_DELTA:u64=0;
// Single CPU, EL1 service executes only between slices; IRQs are masked on
// exception entry. The request queue and completions are fixed-size.
#[derive(Clone,Copy,PartialEq,Eq)]
pub struct Request {pub actor:usize,pub incarnation:u64,pub id:u64,pub save:bool,pub file_op:u8,pub key:[u8;16],pub key_len:usize,pub value:[u8;32],pub value_len:usize}
// Only the routing token is retained for a delayed-reply check; file bytes
// from an exited task must not be retained in this fixture.
#[derive(Clone,Copy)] struct CompletionToken {actor:usize,incarnation:u64,id:u64}
impl Request {fn token(&self)->CompletionToken {CompletionToken{actor:self.actor,incarnation:self.incarnation,id:self.id}}}
#[derive(Clone,Copy,PartialEq,Eq)] struct Completion {actor:usize,incarnation:u64,id:u64,popped:bool,applied:bool,success:bool,oversize:bool,invalid_name:bool,access_denied:bool,write_errno:i64,delivered:bool,consumed:bool,data:[u8;32],data_len:usize}
const EMPTY_COMPLETION:Completion=Completion{actor:0,incarnation:0,id:0,popped:false,applied:false,success:false,oversize:false,invalid_name:false,access_denied:false,write_errno:0,delivered:false,consumed:false,data:[0;32],data_len:0};
struct Ipc {queue:[Option<Request>;2],head:usize,len:usize,completions:[Completion;9],accepted:[u64;8],accepted_len:usize,applied:[u64;8],applied_len:usize,consumed:[u64;8],consumed_len:usize,full:bool,full_consumed:bool,range:[bool;2],overflow:[bool;2],duplicate:[bool;2],utf8:[bool;2],spoof:[bool;2],save_denied:bool,denial_consumed:bool}
static mut IPC:Ipc=Ipc{queue:[None;2],head:0,len:0,completions:[EMPTY_COMPLETION;9],accepted:[0;8],accepted_len:0,applied:[0;8],applied_len:0,consumed:[0;8],consumed_len:0,full:false,full_consumed:false,range:[false;2],overflow:[false;2],duplicate:[false;2],utf8:[false;2],spoof:[false;2],save_denied:false,denial_consumed:false};
// A loaded process can have one generic file operation in flight. Only copied
// bytes and offsets within its private page are retained, never Rust borrows.
#[derive(Clone,Copy,PartialEq,Eq)]
enum UserRegion {Data,Stack}
#[derive(Clone,Copy,PartialEq,Eq)]
struct UserLocation {region:UserRegion,offset:usize}
#[derive(Clone,Copy)]
struct GenericIo {token:CompletionToken,file_op:u8,name:[u8;16],name_len:usize,name_location:UserLocation,dest_location:UserLocation,capacity:usize}
static mut GENERIC_IO:[Option<GenericIo>;2]=[None;2];
static mut NEXT_GENERIC_ID:u64=8;
// Aggregate only; never expose filenames, user addresses or file contents.
static mut GENERIC_READ_SUBMITTED:u64=0;
static mut GENERIC_READ_COMPLETED:u64=0;
static mut GENERIC_WRITE_SUBMITTED:u64=0;
static mut GENERIC_WRITE_COMPLETED:u64=0;
fn clear_generic_io(actor:usize) {
 assert!(actor<2);
 // Writing `None` alone may leave the old Option payload bytes in memory.
 // Volatile-zero the copied name, offsets and token while the variant is
 // still Some, then clear the discriminant. Single CPU, SVC/service only.
 let p=unsafe{addr_of_mut!(GENERIC_IO).cast::<Option<GenericIo>>().add(actor)};
 if let Some(mut old)=unsafe{read_volatile(p)} {
  old.file_op=0;old.name=[0;16];old.name_len=0;old.name_location=UserLocation{region:UserRegion::Data,offset:0};old.dest_location=UserLocation{region:UserRegion::Data,offset:0};old.capacity=0;
  old.token=CompletionToken{actor:0,incarnation:0,id:0};
  unsafe{write_volatile(p,Some(old))};
 }
 unsafe{write_volatile(p,None)};
}
fn completion_slot(actor:usize,id:u64)->Option<usize> {
 if actor>=2 {return None}
 match id {1..=7=>Some((id-1) as usize),8.. =>Some(7+actor),_=>None}
}
fn incarnation(actor:usize)->u64 {
 assert!(actor<2);
 let id=unsafe{read_volatile(addr_of!(TASK_INCARNATIONS).cast::<u64>().add(actor))};
 assert!(id!=0);id
}
fn in_page(p:u64,n:usize)->bool {n>0 && p>=super::mmu::ALIAS as u64 && p.checked_add(n as u64).is_some_and(|end|end<=(super::mmu::ALIAS+PAGE) as u64)}
// Capture only a region and offset. The frame and current EL0 mapping are
// revalidated on each retry, with no guest reference retained across SVCs.
fn user_location(actor:usize,address:u64,len:usize,write:bool,allow_empty:bool)->Option<UserLocation> {
 if actor>=2 || super::mmu::active_root()!=super::mmu::root(actor) || (len==0 && !allow_empty) {return None}
 let end=address.checked_add(u64::try_from(len).ok()?)?;
 for region in [UserRegion::Data,UserRegion::Stack] {
  let (base,frame)=match region {
   UserRegion::Data=>(super::mmu::ALIAS,unsafe{read_volatile(addr_of!(TASK_FRAMES).cast::<usize>().add(actor))}),
   UserRegion::Stack=>(super::mmu::USER_STACK,unsafe{read_volatile(addr_of!(PAIR_STACK_FRAMES).cast::<usize>().add(actor))}),
  };
  let start=base as u64;let limit=start.checked_add(PAGE as u64)?;
  if address<start || end>limit || (len>0 && address==limit) {continue}
  // A zero-byte buffer may point one byte past the mapped region, but still
  // requires the region's first byte to map to this live process's frame.
  let (probe,probe_len)=if len==0 {(start,1)}else{(address,len)};
  if !super::mmu::user_region_matches(probe,probe_len,base,frame,write) {return None}
  return Some(UserLocation{region,offset:usize::try_from(address-start).ok()?})
 }
 None
}
fn user_address(location:UserLocation)->Option<u64> {
 let base=match location.region {UserRegion::Data=>super::mmu::ALIAS,UserRegion::Stack=>super::mmu::USER_STACK};
 Some(u64::try_from(base.checked_add(location.offset)?).ok()?)
}
unsafe fn read64(p:u64)->u64 {let v:u64;asm!("ldr {v}, [{p}]",p=in(reg)p,v=out(reg)v,options(nostack,preserves_flags));v}
unsafe fn write64(p:u64,v:u64){asm!("str {v}, [{p}]",p=in(reg)p,v=in(reg)v,options(nostack,preserves_flags));}
unsafe fn writebyte(p:u64,v:u8){asm!("strb {v:w}, [{p}]",p=in(reg)p,v=in(reg)v as u32,options(nostack,preserves_flags));}
unsafe fn readbyte(p:u64)->u8 {let v:u32;asm!("ldrb {v:w}, [{p}]",p=in(reg)p,v=out(reg)v,options(nostack,preserves_flags));v as u8}
fn descriptor(actor:usize,index:usize,id:u64)->Option<Request> {
 if actor>1 || index>5 {return None}
 let h=(super::mmu::ALIAS+512+index*40) as u64;
 if !in_page(h,40) || unsafe{read64(h+32)}!=id {return None}
 let (kp,kn,vp,vn)=unsafe{(read64(h),read64(h+8),read64(h+16),read64(h+24))};
 if kn==0 || kn>16 || vn>32 || !in_page(kp,kn as usize) || !in_page(vp,vn as usize) {return None}
 let mut req=Request{actor,incarnation:incarnation(actor),id,save:false,file_op:0,key:[0;16],key_len:kn as usize,value:[0;32],value_len:vn as usize};
 for i in 0..req.key_len {req.key[i]=unsafe{readbyte(kp+i as u64)}}
 for i in 0..req.value_len {req.value[i]=unsafe{readbyte(vp+i as u64)}}
 if core::str::from_utf8(&req.key[..req.key_len]).is_err() || core::str::from_utf8(&req.value[..req.value_len]).is_err(){return None}
 Some(req)
}
fn submit(state:&mut Ipc,req:Request)->bool {
 let Some(i)=completion_slot(req.actor,req.id) else{return false};
 if req.actor>=2 || req.incarnation!=incarnation(req.actor) || state.completions[i].id!=0 || state.len==2 || (req.id<=7 && state.accepted_len>=state.accepted.len()) {return false}
 let slot=(state.head+state.len)%2;state.queue[slot]=Some(req);state.len+=1;
 state.completions[i]=Completion{actor:req.actor,incarnation:req.incarnation,id:req.id,..EMPTY_COMPLETION};
 if req.id<=7 {state.accepted[state.accepted_len]=req.id;state.accepted_len+=1;}true
}
pub fn pop_request()->Option<Request> {
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 // Keep both requests in the queue until the guest has actually seen it full.
 if !s.full_consumed || s.len==0 {return None}
 let r=s.queue[s.head].take()?;s.head=(s.head+1)%2;s.len-=1;
 let c=&mut s.completions[completion_slot(r.actor,r.id).unwrap_or_else(||halt())];
 if c.actor!=r.actor || c.incarnation!=r.incarnation || c.id!=r.id || r.incarnation!=incarnation(r.actor) {halt()}
 c.popped=true;Some(r)
}
/// Check a popped request before a service effect. The service is synchronous
/// on one CPU; this is not an atomic lease across future async/SMP execution.
pub fn request_is_live(request:&Request)->bool {
 if request.actor>=2 || request.incarnation!=incarnation(request.actor) {return false}
 let state=unsafe{read_volatile(addr_of!(TASK_STATE).cast::<u64>().add(request.actor))};
 if state!=1 && state!=2 {return false}
 let s=unsafe{&*addr_of!(IPC)};
 let Some(i)=completion_slot(request.actor,request.id) else{return false};let c=&s.completions[i];
 c.actor==request.actor && c.incarnation==request.incarnation && c.id==request.id && c.popped && !c.applied
}
fn apply_completion(s:&mut Ipc,token:CompletionToken,success:bool,data:&[u8])->bool {
 // Replay only a token captured from an actual earlier incarnation, while
 // the new incarnation's request is popped and awaiting a service response.
 if token.actor<2 && token.id>=8 && unsafe{read_volatile(addr_of!(PAIR_ACTIVE))}
  && token.incarnation==incarnation(token.actor) {
  let p=unsafe{addr_of_mut!(PAIR_STALE_TOKENS).cast::<Option<CompletionToken>>().add(token.actor)};
  let flag=unsafe{addr_of_mut!(PAIR_STALE_REJECTED).cast::<bool>().add(token.actor)};
  let current=s.completions[7+token.actor];
  if !unsafe{read_volatile(flag)} && current.actor==token.actor && current.incarnation==token.incarnation && current.id==token.id && current.popped && !current.applied {
   if let Some(stale)=unsafe{read_volatile(p)} {
    let before=s.completions[7+token.actor];
    if stale.incarnation==token.incarnation || apply_completion_inner(s,stale,success,data) || s.completions[7+token.actor]!=before {halt()}
    // Constructed counterfactual: keep the live actor and request ID but use
    // the older incarnation, isolating the generation check from ID changes.
    let old_generation=CompletionToken{actor:token.actor,incarnation:stale.incarnation,id:token.id};
    if apply_completion_inner(s,old_generation,success,data) || s.completions[7+token.actor]!=before {halt()}
    unsafe{write_volatile(flag,true);write_volatile(addr_of_mut!(PAIR_OLD_INCARNATION_REJECTED).cast::<bool>().add(token.actor),true);write_volatile(p,None)};
   }
  }
 }
 apply_completion_inner(s,token,success,data)
}
fn apply_completion_inner(s:&mut Ipc,token:CompletionToken,success:bool,data:&[u8])->bool {
 if token.actor>=2 || token.incarnation!=incarnation(token.actor) || data.len()>32 {return false}
 let Some(i)=completion_slot(token.actor,token.id) else{return false};let c=&mut s.completions[i];
 if c.actor!=token.actor || c.incarnation!=token.incarnation || c.id!=token.id || !c.popped || c.applied || (token.id<=7 && s.applied_len>=s.applied.len()) {return false}
 c.data[..data.len()].copy_from_slice(data);c.data_len=data.len();
 c.applied=true;c.success=success;if token.id<=7 {s.applied[s.applied_len]=token.id;s.applied_len+=1;}true
}
pub fn complete(request:&Request,success:bool)->bool {
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 apply_completion(s,request.token(),success,&[])
}
/// Complete the file operation in the bounded queue. Bytes are copied into
/// kernel-owned completion storage before the caller's buffers can change.
pub fn complete_file(request:&Request,success:bool,data:&[u8])->bool {
 if !(request.id>=8 || request.id==6 || request.id==7) || data.len()>32 || (request.id==6 && !data.is_empty()) || (request.id>=8 && !matches!(request.file_op,1|2)) || (request.id>=8 && request.file_op==1 && !data.is_empty()) {return false}
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 // The stale request is captured from an actual earlier loaded task. Try it
 // while the newer incarnation's same-ID completion is genuinely popped.
 if request.id==6 && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}==3 {
  if let Some(stale)=unsafe{read_volatile(addr_of!(STALE_TOKEN))} {
   let before=s.completions[5];let applied=s.applied;let count=s.applied_len;
   if stale.incarnation==request.incarnation || apply_completion(s,stale,success,data) || s.completions[5]!=before || s.applied!=applied || s.applied_len!=count {halt()}
   unsafe{write_volatile(addr_of_mut!(STALE_GENERATION_REJECTED),true)};
  }
 }
 apply_completion(s,request.token(),success,data)
}
/// Report an existing file whose contents exceed the generic read limit.
pub fn complete_file_oversize(request:&Request)->bool {
 if request.id<8 || request.file_op!=2 {return false}
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 if !apply_completion(s,request.token(),false,&[]) {return false}
 s.completions[7+request.actor].oversize=true;true
}
/// Report an invalid generic filename separately from a missing file.
pub fn complete_file_invalid_name(request:&Request)->bool {
 if request.id<8 || request.file_op!=2 {return false}
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 if !apply_completion(s,request.token(),false,&[]) {return false}
 s.completions[7+request.actor].invalid_name=true;true
}
#[derive(Clone,Copy)]
pub enum WriteError {InvalidName,NoSpace,Io}
/// Complete a generic file write without confusing invalid names, quota and I/O.
pub fn complete_file_write_error(request:&Request,error:WriteError)->bool {
 if request.id<8 || request.file_op!=1 {return false}
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 if !apply_completion(s,request.token(),false,&[]) {return false}
 s.completions[7+request.actor].write_errno=match error {WriteError::InvalidName=>-22,WriteError::NoSpace=>-28,WriteError::Io=>-5};true
}
/// Historical name: the owner enforces the experimental per-actor namespace
/// in both single and pair modes, after checking request liveness.
pub fn is_pair_request(request:&Request)->bool {
 request.actor<2 && request.id>=8 && unsafe{read_volatile(addr_of!(PAIR_ACTIVE))}
  && unsafe{read_volatile(addr_of!(PAIR_INCARNATIONS).cast::<u64>().add(request.actor))}==request.incarnation
}
pub fn complete_file_access_denied(request:&Request)->bool {
 if !is_pair_request(request) || !matches!(request.file_op,1|2) {return false}
 let s=unsafe{&mut *addr_of_mut!(IPC)};
 if !apply_completion(s,request.token(),false,&[]) {return false}
 s.completions[7+request.actor].access_denied=true;true
}
unsafe fn pair_io_count(actor:usize,submitted:bool) {
 if read_volatile(addr_of!(PAIR_ACTIVE)) {
  let base=if submitted {addr_of_mut!(PAIR_IO_SUBMITTED)} else {addr_of_mut!(PAIR_IO_COMPLETED)};
  let p=base.cast::<u64>().add(actor);write_volatile(p,read_volatile(p).saturating_add(1));
 }
}
pub fn ipc_status()->Value {
 let s=unsafe{&*addr_of!(IPC)};
 let initial_finished=s.consumed_len>=4 && s.applied_len>=4 && s.full && s.full_consumed && s.range==[true;2] && s.overflow==[true;2] && s.duplicate==[true;2] && s.utf8==[true;2] && s.spoof==[true;2] && s.save_denied && s.denial_consumed && s.completions[..4].iter().all(|c|c.success);
 // This bit is set only after EL0 has consumed both completions and checked
 // the returned bytes. Completion buffers themselves are revoked at exit.
 let file_finished=unsafe{read_volatile(addr_of!(FILE_EL0_VERIFIED))};
 json!({"finished":initial_finished && file_finished,"initial_finished":initial_finished,"file_finished":file_finished,"capacity":2,"queue_len":s.len,"accepted":&s.accepted[..s.accepted_len],"applied":&s.applied[..s.applied_len],"consumed":&s.consumed[..s.consumed_len],"generic_read_submitted":unsafe{read_volatile(addr_of!(GENERIC_READ_SUBMITTED))},"generic_read_completed":unsafe{read_volatile(addr_of!(GENERIC_READ_COMPLETED))},"generic_write_submitted":unsafe{read_volatile(addr_of!(GENERIC_WRITE_SUBMITTED))},"generic_write_completed":unsafe{read_volatile(addr_of!(GENERIC_WRITE_COMPLETED))},"full_rejected":s.full,"full_consumed":s.full_consumed,"range_rejected":s.range,"overflow_rejected":s.overflow,"duplicate_rejected":s.duplicate,"utf8_rejected":s.utf8,"spoof_rejected":s.spoof,"save_denied":s.save_denied,"denial_consumed":s.denial_consumed,"source":"synthetic descriptors and bytes constructed by EL0"})
}
// The one-CPU service is never active while EL0 executes. Drop only this
// loaded task's queued file requests before returning control to the service.
fn cancel_loaded_requests(s:&mut Ipc) {
 let actor=unsafe{read_volatile(addr_of!(CURRENT))};
 let image=unsafe{read_volatile(addr_of!(IMAGE_COUNT))} as usize;
 if actor!=0 || !unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} || image==0 || image>4 {halt()}
 let original=s.queue;let old_head=s.head;let old_len=s.len;
 s.queue=[None;2];s.head=0;s.len=0;
 for i in 0..old_len {
  if let Some(req)=original[(old_head+i)%2] {
   if req.actor==0 && req.id>=6 {
    let cancelled=unsafe{read_volatile(addr_of!(PENDING_CANCELLED))};
    unsafe{write_volatile(addr_of_mut!(PENDING_CANCELLED),cancelled.checked_add(1).unwrap_or_else(||halt()))};
   } else {s.queue[s.len]=Some(req);s.len+=1;}
  }
 }
}
fn stop_loaded_task(s:&mut Ipc,reason:u8)->u64 {
 let image=unsafe{read_volatile(addr_of!(IMAGE_COUNT))} as usize;
 if !(2..=6).contains(&reason) {halt()}
 cancel_loaded_requests(s);
 unsafe{write_volatile(addr_of_mut!(SPAWN_FAILED),true);write_volatile(addr_of_mut!(IMAGE_OUTCOMES).cast::<u8>().add(image-1),reason)};
 1
}
// SVC 0x140: poll one bounded read. x0=name VA, x1=name length,
// x2=destination VA, x3=capacity; x0 is bytes read or a negative error.
// -11 means retry, -2 unavailable, -34 too small, -14 invalid range,
// -22 inconsistent arguments/encoding, -75 exhausted request IDs.
fn generic_read(f:&mut Frame,s:&mut Ipc,actor:usize) {
 let (name_va,name_len,dest_va,capacity)=(f.regs[0],f.regs[1],f.regs[2],f.regs[3]);
 let result=|f:&mut Frame,v:i64|{f.regs[0]=v as u64};
 if name_len==0 || name_len>16 || capacity==0 || capacity>32 {result(f,-22);return}
 let (name_len,capacity)=(name_len as usize,capacity as usize);
 let Some(name_location)=user_location(actor,name_va,name_len,false,false) else{result(f,-14);return};
 let Some(dest_location)=user_location(actor,dest_va,capacity,true,false) else{result(f,-14);return};
 let mut name=[0u8;16];
 for (i,byte) in name[..name_len].iter_mut().enumerate() {*byte=unsafe{readbyte(name_va+i as u64)}}
 if core::str::from_utf8(&name[..name_len]).is_err() {result(f,-22);return}
 if let Some(pending)=unsafe{read_volatile(addr_of!(GENERIC_IO).cast::<Option<GenericIo>>().add(actor))} {
  if pending.file_op!=2 || pending.token.actor!=actor || pending.token.incarnation!=incarnation(actor)
   || pending.name!=name || pending.name_len!=name_len || pending.name_location!=name_location
   || pending.dest_location!=dest_location || pending.capacity!=capacity {result(f,-22);return}
  let c=&mut s.completions[7+actor];
  if c.id!=pending.token.id || c.actor!=actor || c.incarnation!=pending.token.incarnation {halt()}
  if !c.applied {result(f,-11);return}
  let code=if c.access_denied {-13} else if c.invalid_name {-22} else if c.oversize {-34} else if !c.success {-2} else if c.data_len>capacity {-34} else {
   // Revalidate the current mapping and incarnation immediately before the
   // copy, since only guest offsets were retained while the service ran.
   let Some(dest)=user_address(pending.dest_location) else{result(f,-14);return};
   if incarnation(actor)!=pending.token.incarnation || user_location(actor,dest,pending.capacity,true,false)!=Some(pending.dest_location) {result(f,-14);return}
   for i in 0..c.data_len {unsafe{writebyte(dest+i as u64,c.data[i])}}
   c.data_len as i64
  };
  *c=EMPTY_COMPLETION;clear_generic_io(actor);unsafe{let p=addr_of_mut!(GENERIC_READ_COMPLETED);write_volatile(p,read_volatile(p).saturating_add(1));pair_io_count(actor,false)};result(f,code);return
 }
 let id=unsafe{read_volatile(addr_of!(NEXT_GENERIC_ID))};
 let Some(next)=id.checked_add(1) else{result(f,-75);return};
 let request=Request{actor,incarnation:incarnation(actor),id,save:false,file_op:2,key:name,key_len:name_len,value:[0;32],value_len:0};
 if !submit(s,request) {result(f,-11);return}
 if unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} {unsafe{write_volatile(addr_of_mut!(PAIR_LAST_TOKENS).cast::<Option<CompletionToken>>().add(actor),Some(request.token()))}}
 unsafe{write_volatile(addr_of_mut!(NEXT_GENERIC_ID),next);write_volatile(addr_of_mut!(GENERIC_IO).cast::<Option<GenericIo>>().add(actor),Some(GenericIo{token:request.token(),file_op:2,name,name_len,name_location,dest_location,capacity}));let p=addr_of_mut!(GENERIC_READ_SUBMITTED);write_volatile(p,read_volatile(p).saturating_add(1));pair_io_count(actor,true)};
 result(f,-11);
}
// SVC 0x141: poll one bounded write. x0=name VA, x1=name length,
// x2=source VA, x3=length; x0 is bytes written or a negative error.
// -11 means retry, -14 invalid range, -22 invalid name/arguments,
// -28 catalog full, -5 I/O failure, -75 exhausted request IDs. A write
// whose on-disk outcome becomes uncertain suspends the fixture instead of
// promising that -5 can be delivered before restart.
fn generic_write(f:&mut Frame,s:&mut Ipc,actor:usize) {
 let (name_va,name_len,src_va,len)=(f.regs[0],f.regs[1],f.regs[2],f.regs[3]);
 let result=|f:&mut Frame,v:i64|{f.regs[0]=v as u64};
 if name_len==0 || name_len>16 || len>32 {result(f,-22);return}
 let (name_len,len)=(name_len as usize,len as usize);
 let Some(name_location)=user_location(actor,name_va,name_len,false,false) else{result(f,-14);return};
 let Some(src_location)=user_location(actor,src_va,len,false,true) else{result(f,-14);return};
 let mut name=[0u8;16];
 for (i,byte) in name[..name_len].iter_mut().enumerate() {*byte=unsafe{readbyte(name_va+i as u64)}}
 if core::str::from_utf8(&name[..name_len]).is_err() {result(f,-22);return}
 if let Some(pending)=unsafe{read_volatile(addr_of!(GENERIC_IO).cast::<Option<GenericIo>>().add(actor))} {
  if pending.file_op!=1 || pending.token.actor!=actor || pending.token.incarnation!=incarnation(actor)
   || pending.name!=name || pending.name_len!=name_len || pending.name_location!=name_location
   || pending.dest_location!=src_location || pending.capacity!=len {result(f,-22);return}
  let c=&mut s.completions[7+actor];
  if c.id!=pending.token.id || c.actor!=actor || c.incarnation!=pending.token.incarnation {halt()}
  if !c.applied {result(f,-11);return}
  let code=if c.access_denied {-13} else if c.success {len as i64} else if c.write_errno!=0 {c.write_errno} else {-5};
  *c=EMPTY_COMPLETION;clear_generic_io(actor);unsafe{let p=addr_of_mut!(GENERIC_WRITE_COMPLETED);write_volatile(p,read_volatile(p).saturating_add(1));pair_io_count(actor,false)};result(f,code);return
 }
 let mut value=[0u8;32];
 for (i,byte) in value[..len].iter_mut().enumerate() {*byte=unsafe{readbyte(src_va+i as u64)}}
 let id=unsafe{read_volatile(addr_of!(NEXT_GENERIC_ID))};
 let Some(next)=id.checked_add(1) else{result(f,-75);return};
 let request=Request{actor,incarnation:incarnation(actor),id,save:false,file_op:1,key:name,key_len:name_len,value,value_len:len};
 if !submit(s,request) {result(f,-11);return}
 if unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} {unsafe{write_volatile(addr_of_mut!(PAIR_LAST_TOKENS).cast::<Option<CompletionToken>>().add(actor),Some(request.token()))}}
 unsafe{write_volatile(addr_of_mut!(NEXT_GENERIC_ID),next);write_volatile(addr_of_mut!(GENERIC_IO).cast::<Option<GenericIo>>().add(actor),Some(GenericIo{token:request.token(),file_op:1,name,name_len,name_location,dest_location:src_location,capacity:len}));let p=addr_of_mut!(GENERIC_WRITE_SUBMITTED);write_volatile(p,read_volatile(p).saturating_add(1));pair_io_count(actor,true)};
 result(f,-11);
}
#[no_mangle]
extern "C" fn user_timeout() {
 let s=unsafe{&mut *addr_of_mut!(IPC)};let _=stop_loaded_task(s,5);
}
#[no_mangle]
extern "C" fn user_bad_pc() {
 let s=unsafe{&mut *addr_of_mut!(IPC)};let _=stop_loaded_task(s,2);
}
#[no_mangle]
extern "C" fn ipc_dispatch(frame:*mut Frame,esr:u64)->u64 {
 let f=unsafe{&mut *frame};let actor=unsafe{read_volatile(addr_of!(CURRENT))} as usize;
 if actor>=2 || super::mmu::active_root()!=super::mmu::root(actor) || f.spsr&0xf!=0 {halt()}
 if unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} {
  let len=unsafe{read_volatile(addr_of!(PAIR_CODE_LEN).cast::<u64>().add(actor))};
  let lo=super::mmu::USER_CODE as u64;
  let valid=esr>>26==0x15 && esr&(1<<25)!=0 && f.elr>lo && f.elr<=lo+len && f.elr&3==0;
  if !valid {unsafe{write_volatile(addr_of_mut!(PAIR_OUTCOMES).cast::<u8>().add(actor),2)};return 1}
  match esr&0xffff {
   0x150=>{if unsafe{read_volatile(addr_of!(PAIR_MODE))}!=2 {f.regs[0]=(-38i64) as u64;return 0};unsafe{write_volatile(addr_of_mut!(PAIR_BARRIER_SEEN).cast::<bool>().add(actor),true)};let counts=unsafe{read_volatile(addr_of!(PAIR_IRQS))};let seen=unsafe{read_volatile(addr_of!(PAIR_BARRIER_SEEN))};let frames=unsafe{read_volatile(addr_of!(TASK_FRAMES))};let live=if frames.iter().all(|p|*p!=0) {unsafe{[read_volatile(frames[0] as *const u64),read_volatile(frames[1] as *const u64)]}} else {[0;2]};if counts[0]>0 && counts[1]>0 && seen==[true,true] && frames.iter().all(|p|*p!=0) {unsafe{write_volatile(addr_of_mut!(PAIR_LIVE_MARKERS),live);write_volatile(addr_of_mut!(PAIR_BARRIER_OPEN),true)}};f.regs[0]=if unsafe{read_volatile(addr_of!(PAIR_BARRIER_OPEN))} {0} else {(-11i64) as u64};return 0},
   0x140=>{let state=unsafe{&mut *addr_of_mut!(IPC)};generic_read(f,state,actor);return 0},
   0x141=>{let state=unsafe{&mut *addr_of_mut!(IPC)};generic_write(f,state,actor);return 0},
   0x123=>{unsafe{write_volatile(addr_of_mut!(PAIR_EXIT_CODES).cast::<Option<u64>>().add(actor),Some(f.regs[0]));write_volatile(addr_of_mut!(PAIR_OUTCOMES).cast::<u8>().add(actor),1)};return 1},
   _=>{unsafe{write_volatile(addr_of_mut!(PAIR_OUTCOMES).cast::<u8>().add(actor),2)};return 1},
  }
 }
 let loaded=actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))};
 if loaded && (esr>>26!=0x15 || esr&(1<<25)==0) {
  let s=unsafe{&mut *addr_of_mut!(IPC)};return stop_loaded_task(s,2)
 }
 if esr>>26==0x24 {
  // Only recover the two intentionally placed read probes. All other faults
  // stop this fixture; this is not general process fault recovery.
  let (pc,target)=if actor==0 {(addr_of!(task_a_cross_fault) as u64,addr_of!(task_b_loop) as u64)} else {(addr_of!(task_b_cross_fault) as u64,addr_of!(task_a_loop) as u64)};
  let far:u64;unsafe{asm!("mrs {}, FAR_EL1",out(reg) far)};
  if esr&(1<<25)==0 || esr&0x3f!=0x0f || esr&(1<<6)!=0 || esr&(1<<10)!=0 || f.elr!=pc || far!=target || f.regs[2]!=target {halt()}
  let flag=unsafe{addr_of_mut!(CROSS_READ_DENIED).cast::<bool>().add(actor)};
  if unsafe{read_volatile(flag)} {halt()}
  unsafe{write_volatile(flag,true)};f.elr+=4;f.regs[2]=0;return 0
 }
 if esr>>26!=0x15 || esr&(1<<25)==0 {halt()}
 let (lo,hi)=if actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} {(super::mmu::USER_CODE as u64,super::mmu::USER_CODE as u64+unsafe{read_volatile(addr_of!(SPAWN_CODE_LEN))})} else if actor==0 {(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64)} else {(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64)};
 // ELR after SVC points at the next instruction, which may be exactly one
 // byte past the image when SVC is its last (aligned) instruction.
 if (loaded && (f.elr<=lo || f.elr>hi || f.elr&3!=0)) || (!loaded && (f.elr<lo || f.elr>=hi)) {if loaded {let s=unsafe{&mut *addr_of_mut!(IPC)};return stop_loaded_task(s,2)} else {halt()}}
 if unsafe{read_volatile(addr_of!(SLICE_MODE))}==0 {return 0}
 let s=unsafe{&mut *addr_of_mut!(IPC)};let op=esr&0xffff;
 let stage=unsafe{read_volatile(addr_of!(IMAGE_COUNT))};
 if loaded && ((op==0x130 && stage>3) || (op==0x131 && stage!=1)) {return stop_loaded_task(s,3)}
 if loaded && !matches!(op,0x103|0x105|0x121|0x122|0x123|0x130|0x131|0x140|0x141|0x7bad) {
  if op==0x666 && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}==3 && unsafe{read64((super::mmu::ALIAS+136) as u64)}==1 {
   unsafe{write_volatile(addr_of_mut!(STALE_RESPONSE_BLOCKED),true)};
  }
  return stop_loaded_task(s,3)
 }
 match op {
  0x100 if actor==0 && s.completions[0].id==0 => {if let Some(r)=descriptor(actor,0,1){if !submit(s,r){halt()}}else{halt()}},
  0x102 if actor==0 && s.completions[0].id==1 && s.completions[1].id==0 => {if let Some(r)=descriptor(actor,1,2){if !submit(s,r){halt()}}else{halt()}},
  0x104 if actor==0 && s.completions[1].id==2 && !s.full => {
   let Some(r)=descriptor(actor,4,3) else {halt()};let before=s.queue;let comp=s.completions;let accepted=s.accepted;let count=s.accepted_len;let head=s.head;let len=s.len;
   if len!=2 || submit(s,r) || s.queue!=before || s.completions!=comp || s.accepted!=accepted || s.accepted_len!=count || s.head!=head || s.len!=len {halt()}
   s.full=true;unsafe{write64((super::mmu::ALIAS+72) as u64,3)};
  },
  0x106 if actor==0 && s.full && !s.full_consumed => {if unsafe{read64((super::mmu::ALIAS+64) as u64)}==3 {s.full_consumed=true;unsafe{write64((super::mmu::ALIAS+64) as u64,0);write64((super::mmu::ALIAS+72) as u64,0)}}},
  0x101 if actor==1 && s.completions[0].consumed && s.completions[1].consumed && s.completions[2].id==0 => {if let Some(r)=descriptor(actor,0,3){if !submit(s,r){halt()}}else{halt()}},
  0x100 if actor==1 => s.spoof[actor]=true,
  0x101 if actor==0 => s.spoof[actor]=true,
  0x1fe => {
   let id=if actor==0 {1}else{3};let h=(super::mmu::ALIAS+512+2*40) as u64;
   if unsafe{read64(h)}==(super::mmu::ALIAS+PAGE-1) as u64 && unsafe{read64(h+8)}==4 && descriptor(actor,2,id).is_none(){s.range[actor]=true}else{halt()}
   let h2=(super::mmu::ALIAS+512+3*40) as u64;
   if unsafe{read64(h2)}==u64::MAX-1 && unsafe{read64(h2+8)}==5 && descriptor(actor,3,id).is_none(){s.overflow[actor]=true}else{halt()}
  },
  0x107 => {
   let id=if actor==0 {1}else{3};let c=s.completions[(id-1) as usize];
   if c.id==id && !s.duplicate[actor] {
    let Some(r)=descriptor(actor,0,id) else {halt()};let before=s.queue;let comp=s.completions;let accepted=s.accepted;let count=s.accepted_len;let head=s.head;let len=s.len;
    if submit(s,r) || s.queue!=before || s.completions!=comp || s.accepted!=accepted || s.accepted_len!=count || s.head!=head || s.len!=len {halt()}
    s.duplicate[actor]=true;
   }
  },
  0x108 => {
   let id=if actor==0 {1}else{3};let h=(super::mmu::ALIAS+512+5*40) as u64;
   let before=s.queue;let comp=s.completions;let accepted=s.accepted;let count=s.accepted_len;let head=s.head;let len=s.len;
   if unsafe{read64(h+16)}!=(super::mmu::ALIAS+360) as u64 || unsafe{read64(h+24)}!=1 || unsafe{readbyte((super::mmu::ALIAS+360) as u64)}!=0xff || descriptor(actor,5,id).is_some() || s.queue!=before || s.completions!=comp || s.accepted!=accepted || s.accepted_len!=count || s.head!=head || s.len!=len {halt()}
   s.utf8[actor]=true;
  },
  0x110 if actor==1 && !s.save_denied => {
   // Dedicated one-shot denial response, separate from queued completions.
   let queue=s.queue;let head=s.head;let len=s.len;let completions=s.completions;
   let accepted=s.accepted;let accepted_len=s.accepted_len;
   unsafe{write64((super::mmu::ALIAS+16) as u64,5);write64((super::mmu::ALIAS+24) as u64,4)};
   if s.queue!=queue || s.head!=head || s.len!=len || s.completions!=completions || s.accepted!=accepted || s.accepted_len!=accepted_len {halt()}
   s.save_denied=true;
  },
  0x111 if actor==1 && s.save_denied && !s.denial_consumed => {
   let id=unsafe{read64((super::mmu::ALIAS+32) as u64)};let code=unsafe{read64((super::mmu::ALIAS+40) as u64)};
   if id==5 && code==4 {
    s.denial_consumed=true;
    unsafe{write64((super::mmu::ALIAS+16) as u64,0);write64((super::mmu::ALIAS+24) as u64,0);write64((super::mmu::ALIAS+32) as u64,0);write64((super::mmu::ALIAS+40) as u64,0)};
   }
  },
  0x110 if actor==0 && s.consumed_len==3 && s.completions[3].id==0 => {
   let r=Request{actor,incarnation:incarnation(actor),id:4,save:true,file_op:0,key:[0;16],key_len:0,value:[0;32],value_len:0};
   if !submit(s,r) {halt()}
  },
  0x130 if actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} && s.completions[5].id==0 => {
   let Some(mut r)=descriptor(actor,0,6) else {return stop_loaded_task(s,3)};r.file_op=1;if !submit(s,r){return stop_loaded_task(s,3)}
   if stage==2 {unsafe{write_volatile(addr_of_mut!(STALE_TOKEN),Some(r.token()))}}
  },
  0x131 if actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} && s.completions[5].consumed && s.completions[6].id==0 => {
   let Some(mut r)=descriptor(actor,0,7) else {return stop_loaded_task(s,3)};r.file_op=2;if !submit(s,r){return stop_loaded_task(s,3)}
  },
  0x121 if actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}==1 && s.completions[6].consumed => {
   if f.elr!=super::mmu::USER_CODE as u64+(addr_of!(spawned_after_svc) as u64-addr_of!(spawned_entry) as u64) || unsafe{read64((super::mmu::ALIAS+128) as u64)}!=0xa1a1 || unsafe{read64((super::mmu::ALIAS+848) as u64)}!=1 {return stop_loaded_task(s,3)}
   unsafe{write_volatile(addr_of_mut!(FILE_EL0_VERIFIED),true);write_volatile(addr_of_mut!(SPAWN_EXECUTED),true);write_volatile(addr_of_mut!(IMAGE_OUTCOMES).cast::<u8>(),1)};return 1;
  },
  0x122 if actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}==4 => {
   if f.elr!=super::mmu::USER_CODE as u64+(addr_of!(second_after_svc) as u64-addr_of!(second_entry) as u64) || unsafe{read64((super::mmu::ALIAS+128) as u64)}!=0xa1a1 {return stop_loaded_task(s,3)}
   unsafe{write_volatile(addr_of_mut!(SPAWN_EXECUTED),true);write_volatile(addr_of_mut!(IMAGE_OUTCOMES).cast::<u8>().add(3),1)};return 1;
  },
  // Generic process exit: x0 is the exit status, independent of code address,
  // image number and the demo's file protocol. Pending requests are dropped;
  // already committed service effects are not rolled back.
  0x123 if loaded => {
   let image=usize::from(unsafe{read_volatile(addr_of!(IMAGE_COUNT))});
   if image==0 || image>4 {halt()}
   let exit_code=f.regs[0];
   unsafe{write_volatile(addr_of_mut!(IMAGE_EXIT_CODES).cast::<Option<u64>>().add(image-1),Some(exit_code))};
   if exit_code!=0 {return stop_loaded_task(s,6)}
   cancel_loaded_requests(s);
   unsafe{write_volatile(addr_of_mut!(SPAWN_EXECUTED),true);write_volatile(addr_of_mut!(IMAGE_OUTCOMES).cast::<u8>().add(image-1),1)};
   return 1;
  },
  0x140 if loaded => generic_read(f,s,actor),
  0x141 if loaded => generic_write(f,s,actor),
  0x7bad if loaded => {
   let expected=unsafe{read_volatile(addr_of!(IMAGE_COUNT))}==1 && f.elr==super::mmu::USER_CODE as u64+(addr_of!(spawned_fail_after_svc) as u64-addr_of!(spawned_entry) as u64);
   return stop_loaded_task(s,if expected {4}else{3});
  },
  0x120 if s.consumed_len==4 && s.denial_consumed && s.len==0 => {
   // The task asks to exit only after the integrated work has completed.
   return 1;
  },
  0x103 => {
    if !s.completions.iter().any(|c|c.id<=7 && c.actor==actor && c.incarnation==incarnation(actor) && c.delivered && !c.consumed) {
     if let Some(c)=s.completions.iter_mut().find(|c|c.id!=0 && c.id<=7 && c.actor==actor && c.incarnation==incarnation(actor) && c.applied && !c.delivered) {
      unsafe{
       write64((super::mmu::ALIAS+80) as u64,c.id);write64((super::mmu::ALIAS+112) as u64,if c.success {1}else{2});
       write64((super::mmu::ALIAS+800) as u64,c.data_len as u64);
       for i in 0..32 {writebyte((super::mmu::ALIAS+808+i) as u64,c.data[i])}
      };c.delivered=true;
     }
    }
  },
  0x105 => {
   let observed=unsafe{read64((super::mmu::ALIAS+88) as u64)};let result=unsafe{read64((super::mmu::ALIAS+120) as u64)};
   if let Some(c)=s.completions.iter_mut().find(|c|c.id==observed && c.id!=0 && c.id<=7 && c.actor==actor && c.incarnation==incarnation(actor) && c.delivered && !c.consumed && result==if c.success {1}else{2}) {
    if s.consumed_len>=s.consumed.len() {halt()} c.consumed=true;s.consumed[s.consumed_len]=observed;s.consumed_len+=1;
    unsafe{write64((super::mmu::ALIAS+80) as u64,0);write64((super::mmu::ALIAS+88) as u64,0);write64((super::mmu::ALIAS+112) as u64,0);write64((super::mmu::ALIAS+120) as u64,0)};
   }
  },
  _ if loaded && (op==0x122 || (op==0x121 && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}!=1)) => return stop_loaded_task(s,3),
  _=>{}
 }
 0
}
// Only reached from a saved, full 272-byte IRQ frame; addresses are guest
// values from this synthetic fixture. This path never returns to the guest.
#[no_mangle]
extern "C" fn irq_report(frame:*const Frame,family:u64)->! {
 let f=unsafe{read_volatile(frame)};
 let actor=unsafe{read_volatile(addr_of!(CURRENT))};
 super::write("FAIL IRQ family=");super::write_decimal(family as usize);
 super::write(" current=");super::write_decimal(actor as usize);
 super::write(" ttbr=");super::write_decimal(super::mmu::active_root() as usize);
 super::write(" elr=");super::write_decimal(f.elr as usize);
 super::write(" spsr=");super::write_decimal(f.spsr as usize);
 for i in 0..31 {super::write(" x");super::write_decimal(i);super::write("=");super::write_decimal(f.regs[i] as usize);}
 super::write("\n");loop{unsafe{asm!("wfe",options(nomem,nostack))}}
}
#[no_mangle]
extern "C" fn unexpected_vector()->! {
 let mut elr:u64;let mut spsr:u64;unsafe{asm!("mrs {},ELR_EL1",out(reg) elr);asm!("mrs {},SPSR_EL1",out(reg) spsr);}
 super::write("FAIL IRQ no_frame elr=");super::write_decimal(elr as usize);
 super::write(" spsr=");super::write_decimal(spsr as usize);super::write("\n");
 loop{unsafe{asm!("wfe",options(nomem,nostack))}}
}
// SP, x18, x19..x30; preserved EL1 call continuation.
#[no_mangle] static mut KERNEL_CONTEXT:[u64;14]=[0;14];
extern "C" {
 static __image_start:u8;static __image_end:u8;static __text_end:u8;
 static __usertext_start:u8;static __usertext_split:u8;static __usertext_end:u8;static __userstack_start:u8;static __userstack_end:u8;
 static __stack_bottom:u8;static __stack_top:u8;static vector_table:u8;
 static task_a_loop:u8;static task_a_end:u8;static task_b_loop:u8;static task_b_end:u8;
 static task_a_cross_fault:u8;static task_b_cross_fault:u8;
 static spawned_entry:u8;static spawned_after_svc:u8;static spawned_fail_after_svc:u8;static spawned_end:u8;
 static second_entry:u8;static second_after_svc:u8;static second_end:u8;
 static fault_entry:u8;static fault_end:u8;static badsvc_entry:u8;static badsvc_end:u8;
 static task_a_init:u8;static task_a_init_end:u8;static task_b_init:u8;static task_b_init_end:u8;
 fn start_first(frame:*const Frame)->!;fn resume_graphics()->!;fn slice_probe(frame:*const Frame)->u64;
}
fn halt()->! {loop{unsafe{asm!("wfe",options(nomem,nostack))}}}
fn check(ok:bool,msg:&str){if !ok{super::write("FAIL EL0 ");super::write(msg);super::write("\r\n");halt()}}
fn physical(value:usize)->usize {let out:usize;unsafe{asm!("mov {out}, {value}",out=out(reg) out,value=in(reg) value,options(nomem,nostack,preserves_flags));}out}
pub fn private_stacks()->(usize,usize) {
 let bitmap=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
 check(bitmap==0,"frame pool already initialized");
 for actor in 0..2 {allocate_task_frame(actor);}
 (task_frame(0),task_frame(1))
}
fn allocate_task_frame(actor:usize)->usize {
 check(actor<2 && unsafe{read_volatile(addr_of!(TASK_FRAMES).cast::<usize>().add(actor))}==0,"task already owns a frame");
 let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
 let slot=(0..6).find(|i|used&(1<<i)==0).unwrap_or_else(||halt());
 let frame=unsafe{addr_of_mut!(USER_FRAMES).cast::<UserStack>().add(slot)} as *mut u8;
 for i in 0..PAGE {unsafe{write_volatile(frame.add(i),0)}}
 for i in 0..PAGE {check(unsafe{read_volatile(frame.add(i))}==0,"frame allocation scrub")}
 unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used|(1<<slot));write_volatile(addr_of_mut!(TASK_FRAMES).cast::<usize>().add(actor),frame as usize)};
 frame as usize
}
fn task_frame(actor:usize)->usize {
 check(actor<2,"frame actor");
 let frame=unsafe{read_volatile(addr_of!(TASK_FRAMES).cast::<usize>().add(actor))};
 check(frame!=0,"frame uninitialized");frame
}
fn frame(actor:u64,sp:usize,pc:usize)->Frame {
 let mut f=EMPTY;for i in 0..31 {f.regs[i]=1000+actor*100+i as u64;}
 f.regs[0]=super::mmu::ALIAS as u64;f.regs[1]=if actor==0 {0xa1a1}else{0xb2b2};f.regs[2]=0;f.regs[3]=0;f.regs[4]=0;
 f.regs[19]=0;f.sp=sp as u64;f.elr=pc as u64;
 f.spsr=if actor==0 {0xa000_0340}else{0x5000_0340};f
}

pub fn start()->! {
 let start=physical(addr_of!(__image_start) as usize);let end=physical(addr_of!(__image_end) as usize);
 let text_end=physical(addr_of!(__text_end) as usize);let ut0=physical(addr_of!(__usertext_start) as usize);let split=physical(addr_of!(__usertext_split) as usize);let ut1=physical(addr_of!(__usertext_end) as usize);
 let us0=physical(addr_of!(__userstack_start) as usize);let us1=physical(addr_of!(__userstack_end) as usize);
 let a=physical(task_frame(0));let b=physical(task_frame(1));
 let kb=physical(addr_of!(__stack_bottom) as usize);let kt=physical(addr_of!(__stack_top) as usize);
 let vector=physical(addr_of!(vector_table) as usize);
 let apc=physical(addr_of!(task_a_loop) as usize);let aend=physical(addr_of!(task_a_end) as usize);
 let bpc=physical(addr_of!(task_b_loop) as usize);let bend=physical(addr_of!(task_b_end) as usize);
 let mut sp:usize;let mut sctlr:u64;unsafe{asm!("mov {},sp",out(reg) sp);asm!("mrs {},SCTLR_EL1",out(reg) sctlr);}
 check(sctlr&1!=0 && sctlr&((1<<2)|(1<<12))==0 && sp>kb && sp<=kt && sp%16==0,"MMU or kernel stack");
 check(start<text_end && text_end<=ut0 && ut0<ut1 && us0<us1 && us1==end && kt<=us0,"section bounds");
 check(ut0%PAGE==0 && ut0.checked_add(PAGE)==Some(split) && split.checked_add(PAGE)==Some(ut1) && us0.checked_add(6*PAGE)==Some(us1) && a%PAGE==0 && b%PAGE==0 && a>=us0 && b>=us0 && a+PAGE<=us1 && b+PAGE<=us1 && (a+PAGE<=b || b+PAGE<=a),"EL0 stack bounds");
 check(apc>=ut0 && apc<aend && aend<=split && bpc>=split && bpc<bend && bend<=ut1,"EL0 code bounds");
 check(vector%2048==0 && vector>=start && vector+2048<=text_end,"VBAR bounds");
 check(core::mem::size_of::<Frame>()==272 && core::mem::size_of::<Event>()==56,"frame layout");
 for p in [a as *const u8,b as *const u8] {for i in 0..PAGE {check(unsafe{read_volatile(p.add(i))}==0,"frame not zeroed")}}
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

/// Exact former raw image, for bounded migration of previously saved VFS data.
pub fn legacy_program()->&'static [u8] {
 let start=addr_of!(spawned_entry) as usize;let end=addr_of!(spawned_end) as usize;
 let size=end.checked_sub(start).unwrap_or_else(||halt());
 check(size>0 && size<=392 && size%4==0,"legacy image bounds");
 unsafe{core::slice::from_raw_parts(start as *const u8,size)}
}
/// Build two demonstration ELF files; the loader does not compare against them.
pub fn approved_program(which:usize)->Option<Vec<u8>> {
 let code=if which==0 {legacy_program()} else {
  let (start,end)=match which {
   1=>(addr_of!(fault_entry) as usize,addr_of!(fault_end) as usize),
   2=>(addr_of!(badsvc_entry) as usize,addr_of!(badsvc_end) as usize),
   3=>(addr_of!(second_entry) as usize,addr_of!(second_end) as usize),
   _=>return None,
  };
  let size=end.checked_sub(start)?;unsafe{core::slice::from_raw_parts(start as *const u8,size)}
 };
 super::loader::wrap(code)
}
fn allocate_code_frame()->usize {
 let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
 check(unsafe{read_volatile(addr_of!(CODE_FRAME))}==0,"code frame already assigned");
 let slot=(0..6).find(|i|used&(1<<i)==0).unwrap_or_else(||halt());
 let frame=unsafe{addr_of_mut!(USER_FRAMES).cast::<UserStack>().add(slot)} as *mut u8;
 for i in 0..PAGE {unsafe{write_volatile(frame.add(i),0)}}
 for i in 0..PAGE {check(unsafe{read_volatile(frame.add(i))}==0,"code frame allocation scrub")}
 unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used|(1<<slot));write_volatile(addr_of_mut!(CODE_FRAME),frame as usize)};
 frame as usize
}
fn reclaim_code_frame() {
 let frame=unsafe{read_volatile(addr_of!(CODE_FRAME))};check(frame!=0,"missing code frame");
 super::mmu::unmap_task_code(0,frame);
 unsafe{write_volatile(addr_of_mut!(CODE_ALIAS_REMOVED),true)};
 for i in 0..PAGE {unsafe{write_volatile((frame as *mut u8).add(i),0)}}
 for i in 0..PAGE {check(unsafe{read_volatile((frame as *const u8).add(i))}==0,"code frame scrub")}
 let base=addr_of_mut!(USER_FRAMES) as usize;let offset=frame.checked_sub(base).unwrap_or_else(||halt());
 check(offset%PAGE==0 && offset/PAGE<6,"code frame outside pool");
 let bit=1u8<<(offset/PAGE);let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};check(used&bit!=0,"code frame already free");
 unsafe{write_volatile(addr_of_mut!(CODE_FRAME),0);write_volatile(addr_of_mut!(FRAME_BITMAP),used&!bit);write_volatile(addr_of_mut!(CODE_FRAME_SCRUBBED),true)}
}

/// Request a pause or resumption of future fixture admissions. This is a
/// service-boundary operation on the sole CPU; running work is never killed.
pub fn set_fixture_pause(paused:bool) {unsafe{write_volatile(addr_of_mut!(FIXTURE_PAUSE_REQUESTED),paused)}}
fn external_admission_ready()->bool {
 let ipc=unsafe{&*addr_of!(IPC)};
 let fixture_policy=unsafe{read_volatile(addr_of!(IMAGE_COUNT))}>=4 || unsafe{read_volatile(addr_of!(FIXTURE_PAUSE_REQUESTED))};
 fixture_policy && unsafe{read_volatile(addr_of!(INITIAL_TASKS_RECLAIMED))}
  && !unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} && !unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))}
  && unsafe{read_volatile(addr_of!(TASK_STATE))}==[3,3] && unsafe{read_volatile(addr_of!(FRAME_BITMAP))}==0
  && unsafe{read_volatile(addr_of!(TASK_FRAMES))}==[0,0] && unsafe{read_volatile(addr_of!(PAIR_CODE_FRAMES))}==[0,0] && unsafe{read_volatile(addr_of!(PAIR_STACK_FRAMES))}==[0,0]
  && unsafe{read_volatile(addr_of!(GENERIC_IO))}.iter().all(Option::is_none)
  && ipc.full_consumed && ipc.len==0 && ipc.completions[7..9].iter().all(|c|c.id==0)
  && super::mmu::active_root()==super::mmu::root(0)
}
pub fn fixture_status()->Value {json!({
 "pause_requested":unsafe{read_volatile(addr_of!(FIXTURE_PAUSE_REQUESTED))},
 "fixture_active":!unsafe{read_volatile(addr_of!(INITIAL_TASKS_RECLAIMED))} || unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))},
 "initial_tasks_done":unsafe{read_volatile(addr_of!(INITIAL_TASKS_RECLAIMED))},
 "image_count":unsafe{read_volatile(addr_of!(IMAGE_COUNT))},
 "admission_ready":external_admission_ready()
})}

// A separate bounded experiment on the existing two roots. Each process may
// use the shared synchronous file service, subject to its fixture namespace;
// the barrier is fixture-only and is not a general IPC primitive.
pub fn start_pair(a:&[u8],b:&[u8])->Result<(), &'static str> {start_programs(a,Some(b))}
pub fn start_single(a:&[u8])->Result<(), &'static str> {start_programs(a,None)}
fn start_programs(a:&[u8],b:Option<&[u8]>)->Result<(), &'static str> {
 if unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} {return Err("pair already started")}
 if !external_admission_ready() {return Err("pair requires idle slots")}
 let run_id=unsafe{read_volatile(addr_of!(PAIR_RUN_ID))}.checked_add(1).ok_or("pair run exhausted")?;
 let count=if b.is_some() {2}else{1};
 let error=if count==2 {"invalid pair ELF"} else {"invalid ELF"};
 let a=super::loader::parse(a).ok_or(error)?;
 let b=match b {Some(bytes)=>Some(super::loader::parse(bytes).ok_or(error)?),None=>None};
 // Two-segment images are supported only by independent admission.
 if a.data.is_some() || b.as_ref().is_some_and(|image|image.data.is_some()) {return Err(error)}
 let first=unsafe{read_volatile(addr_of!(NEXT_INCARNATION))};
 let next=first.checked_add(count as u64).ok_or("incarnation exhausted")?;
 let images=[Some(a),b];let mut data=[0usize;2];let mut code=[0usize;2];
 for i in 0..count {
  let image=images[i].as_ref().unwrap_or_else(||halt());
  data[i]=allocate_task_frame(i);
  let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
  let slot=(0..6).find(|n|used&(1<<n)==0).unwrap_or_else(||halt());
  let ptr=unsafe{addr_of_mut!(USER_FRAMES).cast::<UserStack>().add(slot)} as *mut u8;
  for n in 0..PAGE {unsafe{write_volatile(ptr.add(n),0)}}
  for (n,byte) in image.code.iter().enumerate() {unsafe{write_volatile(ptr.add(n),*byte)}}
  code[i]=ptr as usize;
  unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used|(1<<slot))};
  super::mmu::map_task_alias(i,data[i]);super::mmu::map_task_code(i,code[i]);
 }
 check(if count==2 {super::mmu::verify_pair(data,code)} else {super::mmu::verify_single(data[0],code[0])},"loaded mapping verification");
 unsafe {
  write_volatile(addr_of_mut!(A_FRAME),frame(0,super::mmu::ALIAS+PAGE,super::mmu::USER_CODE+images[0].as_ref().unwrap_or_else(||halt()).entry_offset));
  if let Some(image)=images[1].as_ref() {write_volatile(addr_of_mut!(B_FRAME),frame(1,super::mmu::ALIAS+PAGE,super::mmu::USER_CODE+image.entry_offset))}
  write_volatile(addr_of_mut!(PAIR_STALE_TOKENS),read_volatile(addr_of!(PAIR_LAST_TOKENS)));
  write_volatile(addr_of_mut!(PAIR_LAST_TOKENS),[None;2]);
  write_volatile(addr_of_mut!(PAIR_STALE_REJECTED),[false;2]);write_volatile(addr_of_mut!(PAIR_OLD_INCARNATION_REJECTED),[false;2]);
  write_volatile(addr_of_mut!(PAIR_IRQS),[0;2]);
  write_volatile(addr_of_mut!(PAIR_BARRIER_SEEN),[false;2]);write_volatile(addr_of_mut!(PAIR_BARRIER_OPEN),false);
  write_volatile(addr_of_mut!(PAIR_OUTCOMES),[0;2]);write_volatile(addr_of_mut!(PAIR_LIVE_MARKERS),[0;2]);
  write_volatile(addr_of_mut!(PAIR_IO_SUBMITTED),[0;2]);write_volatile(addr_of_mut!(PAIR_IO_COMPLETED),[0;2]);write_volatile(addr_of_mut!(PAIR_IO_CANCELLED),[0;2]);
  write_volatile(addr_of_mut!(PAIR_EXIT_CODES),[None;2]);write_volatile(addr_of_mut!(PAIR_MARKERS),[0;2]);
  write_volatile(addr_of_mut!(PAIR_DATA_SCRUBBED),[false;2]);write_volatile(addr_of_mut!(PAIR_CODE_SCRUBBED),[false;2]);
  write_volatile(addr_of_mut!(PAIR_STACK_SCRUBBED),[false;2]);write_volatile(addr_of_mut!(PAIR_RW_INIT_VERIFIED),[false;2]);write_volatile(addr_of_mut!(PAIR_TWO_SEGMENT),[false;2]);
  write_volatile(addr_of_mut!(PAIR_DONE),false);write_volatile(addr_of_mut!(PAIR_RUN_ID),run_id);
  write_volatile(addr_of_mut!(PAIR_CODE_FRAMES),code);
  write_volatile(addr_of_mut!(PAIR_CODE_LEN),[images[0].as_ref().unwrap_or_else(||halt()).code.len() as u64,images[1].as_ref().map_or(0,|image|image.code.len() as u64)]);
  write_volatile(addr_of_mut!(PAIR_INCARNATIONS),[first,if count==2 {first+1}else{0}]);
  let prior=read_volatile(addr_of!(TASK_INCARNATIONS));
  write_volatile(addr_of_mut!(TASK_INCARNATIONS),[first,if count==2 {first+1}else{prior[1]}]);
  write_volatile(addr_of_mut!(NEXT_INCARNATION),next);
  write_volatile(addr_of_mut!(PAIR_MAPPINGS_VERIFIED),true);write_volatile(addr_of_mut!(SLOT_MAPPINGS_VERIFIED),if count==2 {[true,true]}else{[true,false]});
  write_volatile(addr_of_mut!(CURRENT),0);write_volatile(addr_of_mut!(TASK_STATE),if count==2 {[1,1]}else{[1,3]});
  write_volatile(addr_of_mut!(PAIR_MODE),count as u8);
  write_volatile(addr_of_mut!(PAIR_STARTED),true);write_volatile(addr_of_mut!(PAIR_ACTIVE),true);
 }
 Ok(())
}
// Rollback only a never-published frame, at a masked service boundary.
fn free_unpublished_frame(frame:usize) {
 for i in 0..PAGE {unsafe{write_volatile((frame as *mut u8).add(i),0)}}
 let offset=frame.checked_sub(addr_of_mut!(USER_FRAMES) as usize).unwrap_or_else(||halt());
 check(offset%PAGE==0 && offset/PAGE<6,"rollback frame outside pool");
 let bit=1u8<<(offset/PAGE);let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};check(used&bit!=0,"rollback frame not allocated");
 unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used&!bit)};
}
/// Admit one independent ELF into an actually free slot at an EL1 service
/// boundary. The other slot's frame, registers, scheduling and IPC are intact.
pub fn start_independent(bytes:&[u8])->Result<(usize,u64), &'static str> {
 if super::boot_identity::identifier().is_none() {return Err("boot identity unavailable")}
 let active=unsafe{read_volatile(addr_of!(PAIR_ACTIVE))};
 if active && unsafe{read_volatile(addr_of!(PAIR_MODE))}!=3 {return Err("incompatible active mode")}
 if unsafe{read_volatile(addr_of!(IMAGE_COUNT))}<4 && !unsafe{read_volatile(addr_of!(FIXTURE_PAUSE_REQUESTED))} {return Err("fixture admission not paused")}
 if !unsafe{read_volatile(addr_of!(INITIAL_TASKS_RECLAIMED))} || unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} || super::mmu::active_root()!=super::mmu::root(0) {return Err("process slots unavailable")}
 let states=unsafe{read_volatile(addr_of!(TASK_STATE))};let frames=unsafe{read_volatile(addr_of!(TASK_FRAMES))};let codes=unsafe{read_volatile(addr_of!(PAIR_CODE_FRAMES))};let stacks=unsafe{read_volatile(addr_of!(PAIR_STACK_FRAMES))};
 let ipc=unsafe{&*addr_of!(IPC)};let pending=unsafe{read_volatile(addr_of!(GENERIC_IO))};
 if !ipc.full_consumed {return Err("process slots unavailable")}
 let slot=(0..2).find(|&i|states[i]==3 && frames[i]==0 && codes[i]==0 && stacks[i]==0 && pending[i].is_none()
  && ipc.completions[7+i].id==0 && !ipc.queue.iter().flatten().any(|r|r.actor==i)).ok_or("no free process slot")?;
 let peer=1-slot;
 // A live peer must be a real independent READY task at this boundary. Never
 // infer ownership from an old status entry or reuse a partially cleaned slot.
 if active && (states[peer]!=1 || frames[peer]==0 || codes[peer]==0) {return Err("process slots unavailable")}
 if !active && (states!=[3,3] || frames!=[0,0] || codes!=[0,0] || stacks!=[0,0] || unsafe{read_volatile(addr_of!(FRAME_BITMAP))}!=0 || ipc.len!=0 || ipc.completions[7..9].iter().any(|c|c.id!=0) || pending.iter().any(Option::is_some)) {return Err("process slots unavailable")}
 let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
 if active {
  let base=addr_of_mut!(USER_FRAMES) as usize;
  let bit=|p:usize| -> Option<u8> {let off=p.checked_sub(base)?;if off%PAGE!=0 || off/PAGE>=6 {return None};Some(1u8<<(off/PAGE))};
  let Some(data_bit)=bit(frames[peer]) else{return Err("process slots unavailable")};
  let Some(code_bit)=bit(codes[peer]) else{return Err("process slots unavailable")};
  let mut expected=data_bit|code_bit;
  if stacks[peer]!=0 {let Some(bit)=bit(stacks[peer]) else{return Err("process slots unavailable")};if expected&bit!=0 {return Err("process slots unavailable")};expected|=bit}
  if data_bit==code_bit || used!=expected {return Err("process slots unavailable")}
 }
 let image=super::loader::parse(bytes).ok_or("invalid ELF")?;
 let need=if image.data.is_some() {3}else{2};
 if 6usize.saturating_sub(used.count_ones() as usize)<need {return Err("no free process frames")}
 if !super::mmu::slot_available(slot) {return Err("process mappings unavailable")}
 let incarnation_id=unsafe{read_volatile(addr_of!(NEXT_INCARNATION))};
 let next=incarnation_id.checked_add(1).ok_or("incarnation exhausted")?;
 let run=unsafe{read_volatile(addr_of!(PAIR_RUN_ID))}.checked_add(1).ok_or("pair run exhausted")?;
 let data=allocate_task_frame(slot);
 if let Some(rw)=image.data.as_ref() {
  for (i,byte) in rw.bytes.iter().enumerate() {unsafe{write_volatile((data as *mut u8).add(i),*byte)}}
  for i in 0..rw.mem_size {let expected=if i<rw.bytes.len() {rw.bytes[i]}else{0};check(unsafe{read_volatile((data as *const u8).add(i))}==expected,"RW initialization")}
 }
 let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
 let index=(0..6).find(|i|used&(1<<i)==0).unwrap_or_else(||halt());
 let code=unsafe{addr_of_mut!(USER_FRAMES).cast::<UserStack>().add(index)} as *mut u8;
 for i in 0..PAGE {unsafe{write_volatile(code.add(i),0)}}
 for (i,byte) in image.code.iter().enumerate() {unsafe{write_volatile(code.add(i),*byte)}}
 unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used|(1<<index))};
 let stack=if image.data.is_some() {
  let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};
  let index=(0..6).find(|i|used&(1<<i)==0).unwrap_or_else(||halt());
  let p=unsafe{addr_of_mut!(USER_FRAMES).cast::<UserStack>().add(index)} as *mut u8;
  for i in 0..PAGE {unsafe{write_volatile(p.add(i),0)}}
  unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used|(1<<index))};p as usize
 }else{0};
 super::mmu::map_task_alias(slot,data);super::mmu::map_task_code(slot,code as usize);
 if stack!=0 {super::mmu::map_task_stack(slot,stack)}
 let verified=super::mmu::verify_slot(slot,data,code as usize,if active {Some((frames[peer],codes[peer]))}else{None})
  && if stack!=0 {super::mmu::verify_stack(slot,stack,if active && stacks[peer]!=0 {Some(stacks[peer])}else{None})}
     else if active && stacks[peer]!=0 {super::mmu::verify_stack(peer,stacks[peer],None)}else{true};
 if !verified {
  if stack!=0 {super::mmu::unmap_task_stack(slot,stack)}
  super::mmu::unmap_task_code(slot,code as usize);super::mmu::unmap_task_alias(slot,data);
  for ptr in [data,code as usize,stack] {if ptr!=0 {free_unpublished_frame(ptr)}}
  unsafe{write_volatile(addr_of_mut!(TASK_FRAMES).cast::<usize>().add(slot),0)};
  return Err("mapping verification failed")
 }
 unsafe {
  let stack_top=if stack!=0 {super::mmu::USER_STACK+PAGE}else{super::mmu::ALIAS+PAGE};
  let context=frame(slot as u64,stack_top,super::mmu::USER_CODE+image.entry_offset);
  if slot==0 {write_volatile(addr_of_mut!(A_FRAME),context)}else{write_volatile(addr_of_mut!(B_FRAME),context)}
  let old=read_volatile(addr_of!(PAIR_LAST_TOKENS).cast::<Option<CompletionToken>>().add(slot));
  write_volatile(addr_of_mut!(PAIR_STALE_TOKENS).cast::<Option<CompletionToken>>().add(slot),old);
  write_volatile(addr_of_mut!(PAIR_LAST_TOKENS).cast::<Option<CompletionToken>>().add(slot),None);
  write_volatile(addr_of_mut!(PAIR_STALE_REJECTED).cast::<bool>().add(slot),false);
  write_volatile(addr_of_mut!(PAIR_OLD_INCARNATION_REJECTED).cast::<bool>().add(slot),false);
  write_volatile(addr_of_mut!(PAIR_IRQS).cast::<u64>().add(slot),0);
  write_volatile(addr_of_mut!(PAIR_OUTCOMES).cast::<u8>().add(slot),0);
  write_volatile(addr_of_mut!(PAIR_IO_SUBMITTED).cast::<u64>().add(slot),0);write_volatile(addr_of_mut!(PAIR_IO_COMPLETED).cast::<u64>().add(slot),0);write_volatile(addr_of_mut!(PAIR_IO_CANCELLED).cast::<u64>().add(slot),0);
  write_volatile(addr_of_mut!(PAIR_EXIT_CODES).cast::<Option<u64>>().add(slot),None);
  write_volatile(addr_of_mut!(PAIR_MARKERS).cast::<u64>().add(slot),0);
  write_volatile(addr_of_mut!(PAIR_DATA_SCRUBBED).cast::<bool>().add(slot),false);write_volatile(addr_of_mut!(PAIR_CODE_SCRUBBED).cast::<bool>().add(slot),false);
  write_volatile(addr_of_mut!(PAIR_CODE_FRAMES).cast::<usize>().add(slot),code as usize);
  write_volatile(addr_of_mut!(PAIR_STACK_FRAMES).cast::<usize>().add(slot),stack);
  write_volatile(addr_of_mut!(PAIR_STACK_SCRUBBED).cast::<bool>().add(slot),false);
  write_volatile(addr_of_mut!(PAIR_RW_INIT_VERIFIED).cast::<bool>().add(slot),image.data.is_some());
  write_volatile(addr_of_mut!(PAIR_TWO_SEGMENT).cast::<bool>().add(slot),image.data.is_some());
  write_volatile(addr_of_mut!(PAIR_CODE_LEN).cast::<u64>().add(slot),image.code.len() as u64);
  write_volatile(addr_of_mut!(PAIR_INCARNATIONS).cast::<u64>().add(slot),incarnation_id);
  write_volatile(addr_of_mut!(TASK_INCARNATIONS).cast::<u64>().add(slot),incarnation_id);
  write_volatile(addr_of_mut!(SLOT_MAPPINGS_VERIFIED).cast::<bool>().add(slot),true);
  write_volatile(addr_of_mut!(NEXT_INCARNATION),next);write_volatile(addr_of_mut!(PAIR_RUN_ID),run);
  write_volatile(addr_of_mut!(PAIR_MAPPINGS_VERIFIED),true);write_volatile(addr_of_mut!(PAIR_DONE),false);
  write_volatile(addr_of_mut!(PAIR_STARTED),true);
  if !active {write_volatile(addr_of_mut!(PAIR_BARRIER_SEEN),[false;2]);write_volatile(addr_of_mut!(PAIR_BARRIER_OPEN),false);write_volatile(addr_of_mut!(PAIR_LIVE_MARKERS),[0;2])}
  write_volatile(addr_of_mut!(PAIR_MODE),3);
  write_volatile(addr_of_mut!(TASK_STATE).cast::<u64>().add(slot),1);
  if !active {write_volatile(addr_of_mut!(CURRENT),slot as u64)}
  write_volatile(addr_of_mut!(PAIR_ACTIVE),true);
 }
 Ok((slot,incarnation_id))
}
/// Stop one current independent incarnation at a service boundary. This is
/// synchronous only in this single-CPU guest: it does not roll back file
/// effects that have already been applied by the owner service.
pub fn stop_independent(slot:usize,expected:u64,boot_id:[u8;16])->Result<(), &'static str> {
 let current_boot=super::boot_identity::identifier().ok_or("boot identity unavailable")?;
 if boot_id!=current_boot {return Err("stale boot identity")}
 if slot>=2 {return Err("invalid process slot")}
 let mut daif:u64;let mut timer:u64;
 unsafe{asm!("mrs {},DAIF",out(reg) daif);asm!("mrs {},CNTV_CTL_EL0",out(reg) timer)}
 if daif&(1<<7)==0 || timer&1!=0 || super::mmu::active_root()!=super::mmu::root(0) || unsafe{read_volatile(addr_of!(SLICE_MODE))}!=1 {return Err("process stop requires service boundary")}
 if !unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} || unsafe{read_volatile(addr_of!(PAIR_MODE))}!=3 {return Err("independent process not active")}
 let incarnation=unsafe{read_volatile(addr_of!(TASK_INCARNATIONS).cast::<u64>().add(slot))};
 let reported=unsafe{read_volatile(addr_of!(PAIR_INCARNATIONS).cast::<u64>().add(slot))};
 if expected==0 || expected!=incarnation || expected!=reported {return Err("stale process incarnation")}
 let states=unsafe{read_volatile(addr_of!(TASK_STATE))};
 if states[slot]!=1 {return Err("process is not ready")}
 let peer=1-slot;let current=unsafe{read_volatile(addr_of!(CURRENT))} as usize;
 if current>=2 || states[current]!=1 || !matches!(states[peer],1|3)
  || unsafe{read_volatile(addr_of!(TASK_FRAMES).cast::<usize>().add(slot))}==0
  || unsafe{read_volatile(addr_of!(PAIR_CODE_FRAMES).cast::<usize>().add(slot))}==0 {return Err("process stop requires service boundary")}
 unsafe {
  write_volatile(addr_of_mut!(PAIR_OUTCOMES).cast::<u8>().add(slot),4);
  write_volatile(addr_of_mut!(PAIR_EXIT_CODES).cast::<Option<u64>>().add(slot),None);
  write_volatile(addr_of_mut!(TASK_STATE).cast::<u64>().add(slot),3);
  if current==slot && states[peer]==1 {write_volatile(addr_of_mut!(CURRENT),peer as u64)}
 }
 // Reclaim removes only this generation's still-queued request and its
 // completion; any earlier committed file operation is intentionally kept.
 reclaim_pair(slot);
 unsafe {if slot==0 {write_volatile(addr_of_mut!(A_FRAME),EMPTY)}else{write_volatile(addr_of_mut!(B_FRAME),EMPTY)}};
 Ok(())
}
pub fn pair_status()->Value {json!({
 "boot_identity_ready":super::boot_identity::identifier().is_some(),"mode":match unsafe{read_volatile(addr_of!(PAIR_MODE))} {1=>"single",2=>"pair",3=>"independent",_=>"none"},"started":unsafe{read_volatile(addr_of!(PAIR_STARTED))},"done":unsafe{read_volatile(addr_of!(PAIR_DONE))},"run_id":unsafe{read_volatile(addr_of!(PAIR_RUN_ID))},"stale_completion_rejected":unsafe{read_volatile(addr_of!(PAIR_STALE_REJECTED))},"old_incarnation_same_id_rejected":unsafe{read_volatile(addr_of!(PAIR_OLD_INCARNATION_REJECTED))},
 "incarnations":unsafe{read_volatile(addr_of!(PAIR_INCARNATIONS))},"mappings_verified":unsafe{read_volatile(addr_of!(PAIR_MAPPINGS_VERIFIED))},
 "slot_mappings_verified":unsafe{read_volatile(addr_of!(SLOT_MAPPINGS_VERIFIED))},
 "slot_states":unsafe{read_volatile(addr_of!(TASK_STATE))}.map(|v|match v {1=>"ready",2=>"running",3=>"exited",_=>"invalid"}),
 "io_submitted":unsafe{read_volatile(addr_of!(PAIR_IO_SUBMITTED))},"io_completed":unsafe{read_volatile(addr_of!(PAIR_IO_COMPLETED))},"io_cancelled_before_apply":unsafe{read_volatile(addr_of!(PAIR_IO_CANCELLED))},"irq_counts":unsafe{read_volatile(addr_of!(PAIR_IRQS))},"exit_codes":unsafe{read_volatile(addr_of!(PAIR_EXIT_CODES))},"outcomes":unsafe{read_volatile(addr_of!(PAIR_OUTCOMES))},"barrier_seen":unsafe{read_volatile(addr_of!(PAIR_BARRIER_SEEN))},"live_markers":if unsafe{read_volatile(addr_of!(PAIR_MODE))}==3 {None}else{Some(unsafe{read_volatile(addr_of!(PAIR_LIVE_MARKERS))})},
 "markers":if unsafe{read_volatile(addr_of!(PAIR_MODE))}==3 {None}else{Some(unsafe{read_volatile(addr_of!(PAIR_MARKERS))})},"data_scrubbed":unsafe{read_volatile(addr_of!(PAIR_DATA_SCRUBBED))},
 "code_scrubbed":unsafe{read_volatile(addr_of!(PAIR_CODE_SCRUBBED))},"stack_scrubbed":unsafe{read_volatile(addr_of!(PAIR_STACK_SCRUBBED))},"rw_init_verified":unsafe{read_volatile(addr_of!(PAIR_RW_INIT_VERIFIED))},"two_segment":unsafe{read_volatile(addr_of!(PAIR_TWO_SEGMENT))},"frame_pool_assigned":unsafe{read_volatile(addr_of!(FRAME_BITMAP))}.count_ones()
})}
#[no_mangle]
extern "C" fn pair_irq_check(f:*const Frame)->u64 {
 let actor=unsafe{read_volatile(addr_of!(CURRENT))} as usize;
 if actor>=2 {halt()}
 let frame=unsafe{&*f};let lo=super::mmu::USER_CODE as u64;
 let len=unsafe{read_volatile(addr_of!(PAIR_CODE_LEN).cast::<u64>().add(actor))};
 let p=unsafe{addr_of_mut!(PAIR_IRQS).cast::<u64>().add(actor)};
 let count=unsafe{read_volatile(p)}.checked_add(1).unwrap_or_else(||halt());
 unsafe{write_volatile(p,count)};
 let timeout=count>256 && unsafe{read_volatile(addr_of!(PAIR_MODE))}!=3;
 let stack=unsafe{read_volatile(addr_of!(PAIR_STACK_FRAMES).cast::<usize>().add(actor))};
 let sp_lo=if stack!=0 {super::mmu::USER_STACK}else{super::mmu::ALIAS} as u64;
 if frame.spsr&0xf!=0 || frame.elr<lo || frame.elr>=lo+len || frame.sp<sp_lo || frame.sp>sp_lo+PAGE as u64 || timeout {
  unsafe{write_volatile(addr_of_mut!(PAIR_OUTCOMES).cast::<u8>().add(actor),if timeout {3}else{2})};return 1
 }
 0
}
fn reclaim_pair(actor:usize) {
 // Revoke this incarnation's queued or completed generic operation before
 // returning the private frames. Another actor's request is untouched.
 let state=unsafe{&mut *addr_of_mut!(IPC)};let generation=incarnation(actor);
 let old=state.queue;let head=state.head;let len=state.len;
 state.queue=[None;2];state.head=0;state.len=0;
 for i in 0..len {if let Some(r)=old[(head+i)%2] {
  if r.actor==actor && r.incarnation==generation && r.id>=8 {
   unsafe {let p=addr_of_mut!(PAIR_IO_CANCELLED).cast::<u64>().add(actor);write_volatile(p,read_volatile(p).saturating_add(1))}
  } else {state.queue[state.len]=Some(r);state.len+=1}
 }}
 let c=&mut state.completions[7+actor];
 check(c.id==0 || (c.actor==actor && c.incarnation==generation),"pair completion ownership");
 *c=EMPTY_COMPLETION;clear_generic_io(actor);
 let data=task_frame(actor);let code=unsafe{read_volatile(addr_of!(PAIR_CODE_FRAMES).cast::<usize>().add(actor))};let stack=unsafe{read_volatile(addr_of!(PAIR_STACK_FRAMES).cast::<usize>().add(actor))};
 check(code!=0,"pair missing code");let marker=if unsafe{read_volatile(addr_of!(PAIR_MODE))}==3 {0}else{unsafe{read_volatile(data as *const u64)}};
 if stack!=0 {super::mmu::unmap_task_stack(actor,stack)}
 super::mmu::unmap_task_code(actor,code);super::mmu::unmap_task_alias(actor,data);
 for ptr in [data,code,stack] {if ptr==0 {continue}
  for n in 0..PAGE {unsafe{write_volatile((ptr as *mut u8).add(n),0)}}
  for n in 0..PAGE {check(unsafe{read_volatile((ptr as *const u8).add(n))}==0,"pair frame scrub")}
  let offset=ptr.checked_sub(addr_of_mut!(USER_FRAMES) as usize).unwrap_or_else(||halt());
  check(offset%PAGE==0 && offset/PAGE<6,"pair frame outside pool");
  let bit=1u8<<(offset/PAGE);let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};check(used&bit!=0,"pair frame already free");
  unsafe{write_volatile(addr_of_mut!(FRAME_BITMAP),used&!bit)};
 }
 unsafe {
  write_volatile(addr_of_mut!(TASK_FRAMES).cast::<usize>().add(actor),0);
  write_volatile(addr_of_mut!(PAIR_CODE_FRAMES).cast::<usize>().add(actor),0);
  write_volatile(addr_of_mut!(PAIR_STACK_FRAMES).cast::<usize>().add(actor),0);
  write_volatile(addr_of_mut!(PAIR_STACK_SCRUBBED).cast::<bool>().add(actor),stack!=0);
  write_volatile(addr_of_mut!(PAIR_MARKERS).cast::<u64>().add(actor),marker);
  write_volatile(addr_of_mut!(PAIR_DATA_SCRUBBED).cast::<bool>().add(actor),true);
  write_volatile(addr_of_mut!(PAIR_CODE_SCRUBBED).cast::<bool>().add(actor),true);
  if read_volatile(addr_of!(TASK_FRAMES))==[0,0] {write_volatile(addr_of_mut!(PAIR_ACTIVE),false);write_volatile(addr_of_mut!(PAIR_DONE),true)}
 }
}

// The service runs with IRQs masked. Revoke the alias and invalidate the TLB
// before zeroing and returning the frame to this bounded boot pool.
fn reclaim_exited(actor:usize) {
 if unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} {if unsafe{read_volatile(addr_of!(TASK_FRAMES).cast::<usize>().add(actor))}!=0 {reclaim_pair(actor)};return}
 let p=unsafe{addr_of_mut!(TASK_FRAMES).cast::<usize>().add(actor)};
 let frame=unsafe{read_volatile(p)};if frame==0{return}
 let image_number=if actor==0 && unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} {unsafe{read_volatile(addr_of!(IMAGE_COUNT))}} else {0};
 check(unsafe{read_volatile(addr_of!(TASK_STATE).cast::<u64>().add(actor))}==3,"reclaim running task");
 let marker=unsafe{read_volatile(frame as *const u64)};
 super::mmu::unmap_task_alias(actor,frame);
 unsafe{write_volatile(addr_of_mut!(ALIAS_REMOVED).cast::<bool>().add(actor),true)};
 for i in 0..PAGE {unsafe{write_volatile((frame as *mut u8).add(i),0)}}
 for i in 0..PAGE {check(unsafe{read_volatile((frame as *const u8).add(i))}==0,"frame scrub")}
 unsafe{write_volatile(addr_of_mut!(FRAME_SCRUBBED).cast::<bool>().add(actor),true)};
 let base=addr_of_mut!(USER_FRAMES) as usize;
 let offset=frame.checked_sub(base).unwrap_or_else(||halt());
 check(offset%PAGE==0 && offset/PAGE<6,"frame outside pool");
 let bit=1u8<<(offset/PAGE);
 if actor==0 {
  if unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} {
   check(unsafe{read_volatile(addr_of!(SPAWN_EXECUTED))} ^ unsafe{read_volatile(addr_of!(SPAWN_FAILED))},"spawn exit result");
   // No completion or buffered file bytes may cross an incarnation boundary.
   // Slots 0..4 belong to the earlier fixed tasks and must not be modified.
   let ipc=unsafe{&mut *addr_of_mut!(IPC)};
   check(!ipc.queue.iter().flatten().any(|r|r.actor==0 && r.id>=6),"pending loaded-task request");
   for i in 5..8 {
    check(ipc.completions[i].id==0 || (ipc.completions[i].actor==0 && ipc.completions[i].incarnation==incarnation(0)),"completion belongs to another incarnation");
    ipc.completions[i]=EMPTY_COMPLETION;
    check(ipc.completions[i]==EMPTY_COMPLETION,"completion revoke");
   }
   // Do not retain a prior incarnation's guest offsets or filename.
   clear_generic_io(actor);
   reclaim_code_frame();
   if !unsafe{read_volatile(addr_of!(SPAWN_FAILED))} {
    let count=unsafe{read_volatile(addr_of!(IMAGE_COUNT))};
    if count==1 {unsafe{write_volatile(addr_of_mut!(IMAGE1_EXITED),true)}}
    else if count==4 {unsafe{write_volatile(addr_of_mut!(IMAGE2_EXITED),true)}}
   }
   unsafe{write_volatile(addr_of_mut!(SPAWN_EXITED),true);write_volatile(addr_of_mut!(SPAWN_ACTIVE),false)}
  } else {unsafe{write_volatile(addr_of_mut!(FIRST_FRAME_A),frame)}}
 }
 let used=unsafe{read_volatile(addr_of!(FRAME_BITMAP))};check(used&bit!=0,"frame already free");
 unsafe{write_volatile(addr_of_mut!(EXIT_MARKERS).cast::<u64>().add(actor),marker);write_volatile(p,0);write_volatile(addr_of_mut!(FRAME_BITMAP),used&!bit)}
 if image_number==0 && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}==0
  && unsafe{read_volatile(addr_of!(TASK_FRAMES))}==[0,0]
  && unsafe{read_volatile(addr_of!(FRAME_BITMAP))}==0 {
  unsafe{write_volatile(addr_of_mut!(INITIAL_TASKS_RECLAIMED),true)}
 }
 if image_number==1 {unsafe{write_volatile(addr_of_mut!(IMAGE1_RECLAIMED),true)}}
 else if image_number==2 {unsafe{write_volatile(addr_of_mut!(IMAGE2_RECLAIMED),true)}}
 else if image_number==3 {unsafe{write_volatile(addr_of_mut!(IMAGE3_RECLAIMED),true)}}
 else if image_number==4 {unsafe{write_volatile(addr_of_mut!(IMAGE4_RECLAIMED),true)}}
 if (1..=4).contains(&image_number) {unsafe{
  write_volatile(addr_of_mut!(COMPLETIONS_REVOKED).cast::<bool>().add(image_number as usize-1),true);
  write_volatile(addr_of_mut!(IMAGE_IRQ_X19_MAX_SEEN).cast::<bool>().add(image_number as usize-1),read_volatile(addr_of!(SPAWN_IRQ_X19_MAX_SEEN)));
 }}
}
fn marker(actor:usize)->u64 {
 let frame=unsafe{read_volatile(addr_of!(TASK_FRAMES).cast::<usize>().add(actor))};
 if frame==0 {unsafe{read_volatile(addr_of!(EXIT_MARKERS).cast::<u64>().add(actor))}} else {unsafe{read_volatile(frame as *const u64)}}
}

// Admit a task from the restricted ELF loader after both initial
// tasks have exited. No arbitrary binary loading or dynamic actor creation.
fn admit_spawned_task(program:&[u8],entry_offset:usize) {
 check(unsafe{read_volatile(addr_of!(TASK_STATE))}==[3,3] && unsafe{read_volatile(addr_of!(IMAGE_COUNT))}<4 && !unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))},"invalid spawn admission");
 let process_id=unsafe{read_volatile(addr_of!(NEXT_INCARNATION))};
 let next_process_id=process_id.checked_add(1).unwrap_or_else(||halt());
 let frame_address=allocate_task_frame(0);
 let previous=unsafe{read_volatile(addr_of!(FIRST_FRAME_A))};
 check(previous!=0 && frame_address==previous,"spawn did not reuse first frame");
 // The allocator zeroed and verified the entire reused frame first.
 unsafe{write_volatile(frame_address as *mut u64,0xa1a1)};
 super::mmu::map_task_alias(0,frame_address);
 let code_frame=allocate_code_frame();
 for (i,byte) in program.iter().enumerate() {unsafe{write_volatile((code_frame as *mut u8).add(i),*byte)}}
 for (i,byte) in program.iter().enumerate() {check(unsafe{read_volatile((code_frame as *const u8).add(i))}==*byte,"loaded image copy")}
 super::mmu::map_task_code(0,code_frame);
 let mut child=frame(0,super::mmu::ALIAS+PAGE,super::mmu::USER_CODE+entry_offset);
 child.regs[4]=1;
 unsafe {
  write_volatile(addr_of_mut!(A_FRAME),child);
  write_volatile(addr_of_mut!(FRAME_SCRUBBED).cast::<bool>(),false);
  write_volatile(addr_of_mut!(ALIAS_REMOVED).cast::<bool>(),false);
  write_volatile(addr_of_mut!(CODE_ALIAS_REMOVED),false);
  write_volatile(addr_of_mut!(CODE_FRAME_SCRUBBED),false);
  write_volatile(addr_of_mut!(SPAWN_CODE_LEN),program.len() as u64);
  let count=read_volatile(addr_of!(IMAGE_COUNT));
  write_volatile(addr_of_mut!(TASK_INCARNATIONS).cast::<u64>(),process_id);
  write_volatile(addr_of_mut!(NEXT_INCARNATION),next_process_id);
  write_volatile(addr_of_mut!(IMAGE_INCARNATIONS).cast::<u64>().add(count as usize),process_id);
  write_volatile(addr_of_mut!(IMAGE_COUNT),count+1);
  write_volatile(addr_of_mut!(SPAWN_EXECUTED),false);write_volatile(addr_of_mut!(SPAWN_EXITED),false);write_volatile(addr_of_mut!(SPAWN_FAILED),false);write_volatile(addr_of_mut!(SPAWN_TICKS),0);write_volatile(addr_of_mut!(SPAWN_IRQ_X19_MAX_SEEN),false);
  write_volatile(addr_of_mut!(IMAGE_LOADED),true);
  write_volatile(addr_of_mut!(SPAWN_REUSED),true);
  write_volatile(addr_of_mut!(SPAWN_ACTIVE),true);
  write_volatile(addr_of_mut!(SPAWN_ADMITTED),true);
  write_volatile(addr_of_mut!(CURRENT),0);
  write_volatile(addr_of_mut!(TASK_STATE).cast::<u64>(),1);
 }
}

/// Explicit service-boundary yielding, never arbitrary EL1 preemption.
pub fn prepare_slices() {
 // Test-only fault injection: corrupt x5 in both fixed saved EL0 frames so
 // the next timer IRQ must take the register-diagnostic path. Not in release
 // images unless the explicit feature is enabled.
 #[cfg(feature="irq-diagnostic-test")]
 unsafe {for i in 0..2 {let p=read_volatile(addr_of!(SAVED).cast::<u64>().add(i)) as *mut Frame;(*p).regs[5]=42;}}
 let mut freq:u64;let mut daif:u64;unsafe{asm!("mrs {},CNTFRQ_EL0",out(reg) freq);asm!("mrs {},DAIF",out(reg) daif);}
 check(daif&(1<<7)!=0,"kernel IRQ not masked");let delta=freq/200;check(delta>0 && delta<=i32::MAX as u64,"slice timer frequency");
 // The boot probe returns from task B into EL1. Both tasks are now ready;
 // start_slice is responsible for dispatching one and marking it RUNNING.
 check(unsafe{read_volatile(addr_of!(CURRENT))}==1 && unsafe{read_volatile(addr_of!(TASK_STATE))}==[1,2],"probe task states");
 unsafe {
  write_volatile(addr_of_mut!(TASK_STATE).cast::<u64>().add(1),1);
  write_volatile(addr_of_mut!(SLICE_DELTA),delta);write_volatile(addr_of_mut!(SLICE_MODE),1);
  write_volatile((GICD+0x000) as *mut u32,0);write_volatile((GICC+0x000) as *mut u32,0);
  write_volatile((GICD+0x180) as *mut u32,1<<27);write_volatile((GICD+0x280) as *mut u32,1<<27);
  write_volatile((GICD+0x400+27) as *mut u8,0x80);write_volatile((GICC+0x004) as *mut u32,0xff);
  write_volatile((GICD+0x100) as *mut u32,1<<27);write_volatile((GICD+0x000) as *mut u32,1);write_volatile((GICC+0x000) as *mut u32,1);
  asm!("dsb sy","isb");
 }
}
pub fn run_slice(programs:[Option<&[u8]>;4]) {
 let mut daif:u64;unsafe{asm!("mrs {},DAIF",out(reg) daif);}check(daif&(1<<7)!=0,"service IRQ unmasked");
 check(super::mmu::active_root()==super::mmu::root(0),"service root");
 let mut states_before=unsafe{read_volatile(addr_of!(TASK_STATE))};
 if states_before==[3,3] {
  let index=unsafe{read_volatile(addr_of!(IMAGE_COUNT))} as usize;
  if index>=4 || unsafe{read_volatile(addr_of!(FIXTURE_PAUSE_REQUESTED))} {return}
  let Some(bytes)=programs[index] else {unsafe{write_volatile(addr_of_mut!(LOADER_REJECTED),true)};return};
  let Some(image)=super::loader::parse(bytes) else {unsafe{write_volatile(addr_of_mut!(LOADER_REJECTED),true)};return};
  if image.data.is_some() {unsafe{write_volatile(addr_of_mut!(LOADER_REJECTED),true)};return}
  unsafe{write_volatile(addr_of_mut!(LOADER_REJECTED),false)};
  admit_spawned_task(image.code,image.entry_offset);states_before=unsafe{read_volatile(addr_of!(TASK_STATE))};
 }
 check(states_before.iter().all(|v|*v==1 || *v==3),"invalid service-boundary task states");
 let current=unsafe{read_volatile(addr_of!(CURRENT))};check(current<2 && states_before[current as usize]==1,"slice actor");
 let pair_before=unsafe{read_volatile(addr_of!(PAIR_ACTIVE))};
 let loaded_before=unsafe{read_volatile(addr_of!(SPAWN_ACTIVE))} || pair_before;
 let frame=unsafe{read_volatile(addr_of!(SAVED).cast::<u64>().add(current as usize))} as *const Frame;
 let before=unsafe{read_volatile(addr_of!(SLICE_TOTAL))};let dispatches=unsafe{read_volatile(addr_of!(SLICE_DISPATCHES))};let exits=unsafe{read_volatile(addr_of!(SLICE_EXITS))};let delta=unsafe{read_volatile(addr_of!(SLICE_DELTA))};check(delta!=0,"slice not initialized");

 let counts_before=[unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>())},unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>().add(1))}];
 let allocations_before=super::heap::allocation_attempts();
 let mut sentinel=[0u64;2];let p=sentinel.as_mut_ptr();let ptr=physical(p as usize);
 let bottom=physical(addr_of!(__stack_bottom) as usize);let top=physical(addr_of!(__stack_top) as usize);
 check(ptr>=bottom && ptr.checked_add(16).is_some_and(|v|v<=top),"service sentinel stack");
 unsafe{p.write_volatile(0x1122_3344_5566_7788);p.add(1).write_volatile(0x0102_0304_0506_0708);}
 unsafe {
  write_volatile(addr_of_mut!(SLICE_IRQS),0);asm!("msr CNTV_TVAL_EL0, {}",in(reg) delta);asm!("msr CNTV_CTL_EL0, {}",in(reg) 1u64);asm!("isb");
  check(slice_probe(frame)==1,"kernel call context");
 }
 let after=unsafe{read_volatile(addr_of!(SLICE_TOTAL))};let ticks=after.checked_sub(before).unwrap_or_else(||halt());
 let exited=unsafe{read_volatile(addr_of!(SLICE_EXITS))}.checked_sub(exits).unwrap_or_else(||halt());
 check(ticks<=2 && exited<=1 && (1..=2).contains(&(ticks+exited)),"slice end count");
 let states_after=unsafe{read_volatile(addr_of!(TASK_STATE))};
 check(dispatches.checked_add(ticks+exited)==Some(unsafe{read_volatile(addr_of!(SLICE_DISPATCHES))}) && states_after.iter().all(|v|*v==1 || *v==3),"slice task state transitions");
 let counts_after=[unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>())},unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>().add(1))}];
 let selected=current as usize;
 if !loaded_before {check(counts_after[selected]>counts_before[selected] || states_after[selected]==3,"EL0 slice progress")}
 check(super::heap::allocation_attempts()==allocations_before,"IRQ allocation");
 check(unsafe{read_volatile(p)}==0x1122_3344_5566_7788 && unsafe{read_volatile(p.add(1))}==0x0102_0304_0506_0708,"service stack sentinel");
 let mut timer:u64;unsafe{asm!("mrs {},CNTV_CTL_EL0",out(reg) timer);asm!("mrs {},DAIF",out(reg) daif);}
 check(timer&1==0 && daif&(1<<7)!=0 && super::mmu::active_root()==super::mmu::root(0),"slice timer, IRQ mask or service root");
 if !loaded_before {check(marker(0)==0xa1a1,"private task A data")}
 if !pair_before {check(marker(1)==0xb2b2,"private task B data");}
 for actor in 0..2 {if states_after[actor]==3 {reclaim_exited(actor)}}
}
pub fn status()->Value {
 let done=unsafe{read_volatile(addr_of!(PROBE_DONE))}==1;
 let events:Vec<Value>=(0..8).map(|i|{let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};json!({"task":e.actor,"count":e.count,"next":e.next})}).collect();
 let counts:Vec<u64>=(0..2).map(|i|unsafe{read_volatile(addr_of!(SLICE_COUNTS).cast::<u64>().add(i))}).collect();
 let slice_ticks=unsafe{read_volatile(addr_of!(SLICE_TOTAL))};
 let values=if unsafe{read_volatile(addr_of!(PAIR_ACTIVE))} && unsafe{read_volatile(addr_of!(PAIR_MODE))}==3 {None}else{Some([marker(0),marker(1)])};
 let states=unsafe{read_volatile(addr_of!(TASK_STATE))}.map(|s|match s {1=>"ready",2=>"running",3=>"exited",_=>"invalid"});
 let outcomes=unsafe{read_volatile(addr_of!(IMAGE_OUTCOMES))}.map(|v|match v {0=>"pending",1=>"success",2=>"fault",3=>"bad_svc",4=>"file_error",5=>"timeout",6=>"exit_error",_=>"invalid"});
 let reclaimed=[unsafe{read_volatile(addr_of!(IMAGE1_RECLAIMED))},unsafe{read_volatile(addr_of!(IMAGE2_RECLAIMED))},unsafe{read_volatile(addr_of!(IMAGE3_RECLAIMED))},unsafe{read_volatile(addr_of!(IMAGE4_RECLAIMED))}];
 json!({"validated":done,"ticks":8,"el":"EL0","events":events,"timer":"disabled","interrupts":"masked","slice_ticks":slice_ticks,"slice_counts":counts,"mode":"service-boundary","private_values":values,"service_asid":super::mmu::active_root()>>48,"cross_code_read_denied":unsafe{read_volatile(addr_of!(CROSS_READ_DENIED))},"task_states":states,"frame_scrubbed":unsafe{read_volatile(addr_of!(FRAME_SCRUBBED))},"alias_removed":unsafe{read_volatile(addr_of!(ALIAS_REMOVED))},"dispatches":unsafe{read_volatile(addr_of!(SLICE_DISPATCHES))},"loader_rejected":unsafe{read_volatile(addr_of!(LOADER_REJECTED))},"image_loaded":unsafe{read_volatile(addr_of!(IMAGE_LOADED))},"code_alias_removed":unsafe{read_volatile(addr_of!(CODE_ALIAS_REMOVED))},"code_frame_scrubbed":unsafe{read_volatile(addr_of!(CODE_FRAME_SCRUBBED))},"image_outcomes":outcomes,"image_exit_codes":unsafe{read_volatile(addr_of!(IMAGE_EXIT_CODES))},"image_irq_x19_max_seen":unsafe{read_volatile(addr_of!(IMAGE_IRQ_X19_MAX_SEEN))},"images_reclaimed":reclaimed,"completions_revoked":unsafe{read_volatile(addr_of!(COMPLETIONS_REVOKED))},"pending_cancelled":unsafe{read_volatile(addr_of!(PENDING_CANCELLED))},"stale_response_blocked":unsafe{read_volatile(addr_of!(STALE_RESPONSE_BLOCKED))},"image_incarnations":unsafe{read_volatile(addr_of!(IMAGE_INCARNATIONS))},"stale_generation_rejected":unsafe{read_volatile(addr_of!(STALE_GENERATION_REJECTED))},"image_count":unsafe{read_volatile(addr_of!(IMAGE_COUNT))},"image1_exited":unsafe{read_volatile(addr_of!(IMAGE1_EXITED))},"image2_exited":unsafe{read_volatile(addr_of!(IMAGE2_EXITED))},"image1_reclaimed":unsafe{read_volatile(addr_of!(IMAGE1_RECLAIMED))},"image2_reclaimed":unsafe{read_volatile(addr_of!(IMAGE2_RECLAIMED))},"spawn_admitted":unsafe{read_volatile(addr_of!(SPAWN_ADMITTED))},"spawn_executed":unsafe{read_volatile(addr_of!(SPAWN_EXECUTED))},"spawn_failed":unsafe{read_volatile(addr_of!(SPAWN_FAILED))},"spawn_exited":unsafe{read_volatile(addr_of!(SPAWN_EXITED))},"spawn_interrupts":unsafe{read_volatile(addr_of!(SPAWN_TICKS))},"spawn_irq_x19_max_seen":unsafe{read_volatile(addr_of!(SPAWN_IRQ_X19_MAX_SEEN))},"spawn_reused_frame":unsafe{read_volatile(addr_of!(SPAWN_REUSED))},"frame_pool_capacity":6,"frame_pool_assigned":unsafe{read_volatile(addr_of!(FRAME_BITMAP))}.count_ones()})
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
    adrp x7, PAIR_ACTIVE
    add x7, x7, :lo12:PAIR_ACTIVE
    ldrb w7, [x7]
    cbz w7, slice_not_pair
    mov x0, sp
    bl pair_irq_check
    cbnz x0, svc_exit
    adrp x0, CURRENT
    add x0, x0, :lo12:CURRENT
    ldr x2, [x0]
    b slice_count_update
slice_not_pair:
    // A loaded task has arbitrary integer registers and flags. Keep the
    // kernel/TTBR checks above; enforce a bounded execution budget and its
    // code mapping without treating user registers as kernel invariants.
    adrp x7, SPAWN_ACTIVE
    add x7, x7, :lo12:SPAWN_ACTIVE
    ldrb w7, [x7]
    cbz w7, slice_fixed_registers
    ldr x8, [sp, #152]       // saved user x19, diagnostic only
    cmn x8, #1
    b.ne 24f
    adrp x7, SPAWN_IRQ_X19_MAX_SEEN
    add x7, x7, :lo12:SPAWN_IRQ_X19_MAX_SEEN
    mov w8, #1
    strb w8, [x7]
24:
    adrp x7, SPAWN_TICKS
    add x7, x7, :lo12:SPAWN_TICKS
    ldr x8, [x7]
    add x8, x8, #1
    str x8, [x7]
    cmp x8, #256
    b.ls slice_loaded_pc
    bl user_timeout
    b svc_exit
slice_loaded_pc:
    ldr x8, [sp, #256]
    movz x9, #0x5000, lsl #16
    add x9, x9, #0x1000
    adrp x10, SPAWN_CODE_LEN
    add x10, x10, :lo12:SPAWN_CODE_LEN
    ldr x10, [x10]
    add x10, x9, x10
    cmp x8, x9
    b.lo slice_loaded_bad_pc
    cmp x8, x10
    b.hs slice_loaded_bad_pc
    b slice_count_update
slice_loaded_bad_pc:
    bl user_bad_pc
    b svc_exit
slice_fixed_registers:
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
    ldr x8, [sp, #256]
    cbnz x2, slice_init_b
    adrp x9, task_a_init
    add x9, x9, :lo12:task_a_init
    adrp x10, task_a_init_end
    add x10, x10, :lo12:task_a_init_end
    b slice_init_range
slice_init_b:
    adrp x9, task_b_init
    add x9, x9, :lo12:task_b_init
    adrp x10, task_b_init_end
    add x10, x10, :lo12:task_b_init_end
slice_init_range:
    cmp x8, x9
    b.lo slice_non_init
    cmp x8, x10
    b.hs slice_non_init
    ldr x8, [sp, #24]
    cbnz x8, irq_register_fail
    b slice_after_temps
slice_non_init:
    cbnz x2, 23f
    ldr x8, [sp, #256]
    movz x9, #0x5000, lsl #16
    add x9, x9, #0x1000
    adrp x10, SPAWN_CODE_LEN
    add x10, x10, :lo12:SPAWN_CODE_LEN
    ldr x10, [x10]
    add x10, x9, x10
    cmp x8, x9
    b.lo 23f
    cmp x8, x10
    b.lo slice_after_temps
23: ldr x8, [sp, #16]
    cmp x8, #5
    b.ls slice_x2_ok
    cmp x8, x7
    b.ne irq_register_fail
slice_x2_ok:
    ldr x8, [sp, #24]
    cmp x8, #7
    b.hi irq_register_fail
slice_after_temps:
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
    adrp x11, SPAWN_ACTIVE
    add x11, x11, :lo12:SPAWN_ACTIVE
    ldrb w11, [x11]
    cbz w11, 26f
    movz x7, #0x5000, lsl #16
    add x7, x7, #0x1000
    adrp x9, SPAWN_CODE_LEN
    add x9, x9, :lo12:SPAWN_CODE_LEN
    ldr x9, [x9]
    add x9, x7, x9
    b 27f
26: adrp x7, task_a_loop
    add x7, x7, :lo12:task_a_loop
    adrp x9, task_a_end
    add x9, x9, :lo12:task_a_end
27: mov x10, #0xa0000000
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
slice_count_update:
    adrp x5, SLICE_COUNTS
    add x5, x5, :lo12:SLICE_COUNTS
    // Scheduler accounting belongs to EL1. A loaded program controls x19
    // and must never be able to overwrite the kernel's progress counter.
    ldr x6, [x5, x2, lsl #3]
    adds x6, x6, #1
    b.cs irq_schedule_fail
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
    // Interrupt deschedules RUNNING -> READY. Only a READY task may be
    // dispatched; at the second interrupt we return to EL1 with both READY.
    adrp x4, TASK_STATE
    add x4, x4, :lo12:TASK_STATE
    ldr x5, [x4, x2, lsl #3]
    cmp x5, #2
    b.ne irq_schedule_fail
    mov x5, #1
    str x5, [x4, x2, lsl #3]
    eor x2, x2, #1
    ldr x5, [x4, x2, lsl #3]
    cmp x5, #1
    b.eq 21f
    // The other task has exited: continue this READY task if necessary.
    cmp x5, #3
    b.ne irq_schedule_fail
    eor x2, x2, #1
    ldr x5, [x4, x2, lsl #3]
    cmp x5, #1
    b.ne irq_schedule_fail
21: str x2, [x0]
    cmp x6, #2
    b.eq slice_return_kernel
    mov x5, #2
    str x5, [x4, x2, lsl #3]
    adrp x4, SLICE_DISPATCHES
    add x4, x4, :lo12:SLICE_DISPATCHES
    ldr x5, [x4]
    add x5, x5, #1
    str x5, [x4]
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
    // READY -> RUNNING before entering EL0. This is authoritative: a task
    // in any other state cannot be selected by this scheduler.
    adrp x11, TASK_STATE
    add x11, x11, :lo12:TASK_STATE
    ldr x12, [x11, x10, lsl #3]
    cmp x12, #1
    b.ne irq_fatal
    mov x12, #2
    str x12, [x11, x10, lsl #3]
    adrp x11, SLICE_DISPATCHES
    add x11, x11, :lo12:SLICE_DISPATCHES
    ldr x12, [x11]
    add x12, x12, #1
    str x12, [x11]
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
    cbnz x0, svc_exit
    mov x0, sp
    add sp, sp, #272
    b svc_restore_frame
svc_exit:
    // Exit is immediate: never restore this EL0 frame. Stop the timer and
    // clear its pending interrupt, retire the current task, then return EL1.
    msr CNTV_CTL_EL0, xzr
    isb
    mov x9, #0x08000000
    mov w10, #0x08000000
    str w10, [x9, #0x280]
    dsb sy
    adrp x9, CURRENT
    add x9, x9, :lo12:CURRENT
    ldr x10, [x9]
    cmp x10, #1
    b.hi irq_fatal
    adrp x11, TASK_STATE
    add x11, x11, :lo12:TASK_STATE
    ldr x12, [x11, x10, lsl #3]
    cmp x12, #2
    b.ne irq_fatal
    mov x12, #3
    str x12, [x11, x10, lsl #3]
    eor x10, x10, #1
    ldr x12, [x11, x10, lsl #3]
    cmp x12, #1
    b.ne 22f
    str x10, [x9]             // next service slice uses the remaining task
22: adrp x11, SLICE_EXITS
    add x11, x11, :lo12:SLICE_EXITS
    ldr x12, [x11]
    add x12, x12, #1
    str x12, [x11]
    b slice_return_kernel     // releases this 272-byte frame on return
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

    .section .usertext.a, "ax"
    .global task_a_loop
    .global task_a_end
task_a_loop:
    .global task_a_init
    .global task_a_init_end
task_a_init:
    ldr x2, [x0, #48]
    cbnz x2, task_a_init_restore
    adrp x2, task_b_loop
    add x2, x2, :lo12:task_b_loop
    .global task_a_cross_fault
task_a_cross_fault:
    ldr x2, [x2]   // privileged page in A's translation root
    movz x2, #1
    str x2, [x0, #56]
    movz x2, #26740
    movk x2, #28005, lsl #16
    movk x2, #101, lsl #32
    str x2, [x0, #256]
    movz x2, #30056
    movk x2, #24941, lsl #16
    movk x2, #11630, lsl #32
    movk x2, #28271, lsl #48
    str x2, [x0, #288]
    movz x2, #101
    str x2, [x0, #296]
    movz x2, #30056
    movk x2, #24941, lsl #16
    movk x2, #11630, lsl #32
    movk x2, #30580, lsl #48
    str x2, [x0, #320]
    movz x2, #111
    str x2, [x0, #328]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #512]
    movz x2, #5
    str x2, [x0, #520]
    movz x2, #288
    movk x2, #20480, lsl #16
    str x2, [x0, #528]
    movz x2, #9
    str x2, [x0, #536]
    movz x2, #1
    str x2, [x0, #544]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #552]
    movz x2, #5
    str x2, [x0, #560]
    movz x2, #320
    movk x2, #20480, lsl #16
    str x2, [x0, #568]
    movz x2, #9
    str x2, [x0, #576]
    movz x2, #2
    str x2, [x0, #584]
    movz x2, #4095
    movk x2, #20480, lsl #16
    str x2, [x0, #592]
    movz x2, #4
    str x2, [x0, #600]
    movz x2, #288
    movk x2, #20480, lsl #16
    str x2, [x0, #608]
    movz x2, #9
    str x2, [x0, #616]
    movz x2, #1
    str x2, [x0, #624]
    movz x2, #65534
    movk x2, #65535, lsl #16
    movk x2, #65535, lsl #32
    movk x2, #65535, lsl #48
    str x2, [x0, #632]
    movz x2, #5
    str x2, [x0, #640]
    movz x2, #288
    movk x2, #20480, lsl #16
    str x2, [x0, #648]
    movz x2, #9
    str x2, [x0, #656]
    movz x2, #1
    str x2, [x0, #664]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #672]
    movz x2, #5
    str x2, [x0, #680]
    movz x2, #320
    movk x2, #20480, lsl #16
    str x2, [x0, #688]
    movz x2, #9
    str x2, [x0, #696]
    movz x2, #3
    str x2, [x0, #704]
    movz x2, #255
    str x2, [x0, #360]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #712]
    movz x2, #5
    str x2, [x0, #720]
    movz x2, #360
    movk x2, #20480, lsl #16
    str x2, [x0, #728]
    movz x2, #1
    str x2, [x0, #736]
    movz x2, #1
    str x2, [x0, #744]
    movz x2, #1
    str x2, [x0, #48]
task_a_init_restore:
    ldr x2, [x0]
    mov x3, #0
task_a_init_end:
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
    svc #0x108
    svc #0x100
    svc #0x102
    svc #0x104
    // EL0 compares the full-queue response to E_FULL=3 before echoing it.
    ldr x2, [x0, #72]
    cbz x2, 9f
    eor x3, x2, #3
    cbnz x3, el0_failure_a
    str x2, [x0, #64]
9:  ldr x2, [x0]
    mov x3, #0
    svc #0x106
    svc #0x101
    svc #0x107
    svc #0x110
    svc #0x103
    // Each task compares the delivered ID with its next expected ID and
    // the result with success=1 before echoing both to the kernel.
    ldr x2, [x0, #80]
    cbz x2, 10f
    ldr x3, [x0, #56]
    eor x3, x3, x2
    cbnz x3, el0_failure_a
    ldr x3, [x0, #112]
    eor x3, x3, #1
    cbnz x3, el0_failure_a
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
    svc #0x120
    svc #0x1ff
    add x19, x19, #1
    b task_a_loop
el0_failure_a:
    svc #0x7bad
    b el0_failure_a
    .global spawned_entry
    .global spawned_after_svc
spawned_entry:
    str x1, [x0, #128]
    movz x2, #0x6f6e
    movk x2, #0x6574, lsl #16
    str x2, [x0, #256]       // name: note
    movz x2, #0x6568
    movk x2, #0x6c6c, lsl #16
    movk x2, #0x6f, lsl #32
    str x2, [x0, #288]       // data: hello
    add x2, x0, #256
    str x2, [x0, #512]
    mov x2, #4
    str x2, [x0, #520]
    add x2, x0, #288
    str x2, [x0, #528]
    mov x2, #5
    str x2, [x0, #536]
    mov x2, #6
    str x2, [x0, #56]
spawned_loop:
    // The descriptor ID changes only after the PUT completion was consumed.
    ldr x2, [x0, #56]
    str x2, [x0, #544]
    mov x2, x1
    mov x3, #0
    svc #0x130
    svc #0x131
    svc #0x103
    ldr x2, [x0, #80]
    cbz x2, 24f
    ldr x3, [x0, #56]
    eor x3, x3, x2
    cbnz x3, spawned_fail
    ldr x3, [x0, #112]
    eor x3, x3, #1
    cbnz x3, spawned_fail
    eor x3, x2, #7
    cbnz x3, 25f
    ldr x3, [x0, #800]
    mov x2, #5
    eor x3, x3, x2
    cbnz x3, spawned_fail
    ldr x3, [x0, #808]
    movz x2, #0x6568
    movk x2, #0x6c6c, lsl #16
    movk x2, #0x6f, lsl #32
    eor x3, x3, x2
    cbnz x3, spawned_fail
    mov x2, #1
    str x2, [x0, #848]       // EL0 verified length and bytes itself
25: ldr x2, [x0, #80]
    str x2, [x0, #88]
    ldr x3, [x0, #112]
    str x3, [x0, #120]
    ldr x2, [x0, #56]
    add x2, x2, #1
    str x2, [x0, #56]
24: mov x2, x1
    mov x3, #0
    svc #0x105
    svc #0x121               // rejected until GET is consumed
spawned_after_svc:
    add x19, x19, #1
    b spawned_loop           // never executed after accepted exit
spawned_fail:
    svc #0x7bad
    .global spawned_fail_after_svc
spawned_fail_after_svc:
    b spawned_fail
    .global spawned_end
spawned_end:
    .global second_entry
    .global second_after_svc
    .global second_end
second_entry:
    str x1, [x0, #128]
    svc #0x122
second_after_svc:
    b second_after_svc
second_end:
    .global fault_entry
    .global fault_end
fault_entry:
    // Queue a real file request, then fault. Cleanup must cancel/revoke it
    // before the next incarnation can inspect a completion.
    movz x2, #0x6f6e
    movk x2, #0x6574, lsl #16
    str x2, [x0, #256]
    movz x2, #0x6568
    movk x2, #0x6c6c, lsl #16
    movk x2, #0x6f, lsl #32
    str x2, [x0, #288]
    add x2, x0, #256
    str x2, [x0, #512]
    mov x2, #4
    str x2, [x0, #520]
    add x2, x0, #288
    str x2, [x0, #528]
    mov x2, #5
    str x2, [x0, #536]
    mov x2, #6
    str x2, [x0, #544]
    svc #0x130
    mov x2, xzr
    ldr x2, [x2]      // unmapped virtual address: real EL0 data abort
    b fault_entry
fault_end:
    .global badsvc_entry
    .global badsvc_end
badsvc_entry:
    svc #0x103        // must not receive the prior incarnation's reply
    ldr x2, [x0, #80]
    cbnz x2, 28f
    mov x2, #1
    str x2, [x0, #136]
    // Submit the same request ID under a new incarnation and wait until the
    // service completes it. The service attempts the genuine stale request
    // before applying this completion; the old generation must be rejected.
    movz x2, #0x6f6e
    movk x2, #0x6574, lsl #16
    str x2, [x0, #256]
    movz x2, #0x6568
    movk x2, #0x6c6c, lsl #16
    movk x2, #0x6f, lsl #32
    str x2, [x0, #288]
    add x2, x0, #256
    str x2, [x0, #512]
    mov x2, #4
    str x2, [x0, #520]
    add x2, x0, #288
    str x2, [x0, #528]
    mov x2, #5
    str x2, [x0, #536]
    mov x2, #6
    str x2, [x0, #544]
    svc #0x130
29: svc #0x103
    ldr x2, [x0, #80]
    cbz x2, 29b
    str x2, [x0, #88]
    ldr x3, [x0, #112]
    str x3, [x0, #120]
    svc #0x105
    svc #0x666        // denied by the loaded-task syscall allowlist
28: svc #0x667
    b badsvc_entry
badsvc_end:
task_a_end:
    .section .usertext.b, "ax"
    .global task_b_loop
    .global task_b_end
task_b_loop:
    .global task_b_init
    .global task_b_init_end
task_b_init:
    ldr x2, [x0, #48]
    cbnz x2, task_b_init_restore
    adrp x2, task_a_loop
    add x2, x2, :lo12:task_a_loop
    .global task_b_cross_fault
task_b_cross_fault:
    ldr x2, [x2]   // privileged page in B's translation root
    movz x2, #3
    str x2, [x0, #56]
    movz x2, #24940
    movk x2, #28537, lsl #16
    movk x2, #29813, lsl #32
    str x2, [x0, #256]
    movz x2, #29537
    movk x2, #26995, lsl #16
    movk x2, #29811, lsl #32
    movk x2, #28257, lsl #48
    str x2, [x0, #288]
    movz x2, #116
    str x2, [x0, #296]
    movz x2, #29537
    movk x2, #26995, lsl #16
    movk x2, #29811, lsl #32
    movk x2, #28257, lsl #48
    str x2, [x0, #320]
    movz x2, #116
    str x2, [x0, #328]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #512]
    movz x2, #6
    str x2, [x0, #520]
    movz x2, #288
    movk x2, #20480, lsl #16
    str x2, [x0, #528]
    movz x2, #9
    str x2, [x0, #536]
    movz x2, #3
    str x2, [x0, #544]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #552]
    movz x2, #6
    str x2, [x0, #560]
    movz x2, #320
    movk x2, #20480, lsl #16
    str x2, [x0, #568]
    movz x2, #9
    str x2, [x0, #576]
    movz x2, #3
    str x2, [x0, #584]
    movz x2, #4095
    movk x2, #20480, lsl #16
    str x2, [x0, #592]
    movz x2, #4
    str x2, [x0, #600]
    movz x2, #288
    movk x2, #20480, lsl #16
    str x2, [x0, #608]
    movz x2, #9
    str x2, [x0, #616]
    movz x2, #3
    str x2, [x0, #624]
    movz x2, #65534
    movk x2, #65535, lsl #16
    movk x2, #65535, lsl #32
    movk x2, #65535, lsl #48
    str x2, [x0, #632]
    movz x2, #5
    str x2, [x0, #640]
    movz x2, #288
    movk x2, #20480, lsl #16
    str x2, [x0, #648]
    movz x2, #9
    str x2, [x0, #656]
    movz x2, #3
    str x2, [x0, #664]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #672]
    movz x2, #6
    str x2, [x0, #680]
    movz x2, #320
    movk x2, #20480, lsl #16
    str x2, [x0, #688]
    movz x2, #9
    str x2, [x0, #696]
    movz x2, #3
    str x2, [x0, #704]
    movz x2, #255
    str x2, [x0, #360]
    movz x2, #256
    movk x2, #20480, lsl #16
    str x2, [x0, #712]
    movz x2, #6
    str x2, [x0, #720]
    movz x2, #360
    movk x2, #20480, lsl #16
    str x2, [x0, #728]
    movz x2, #1
    str x2, [x0, #736]
    movz x2, #3
    str x2, [x0, #744]
    movz x2, #1
    str x2, [x0, #48]
task_b_init_restore:
    ldr x2, [x0]
    mov x3, #0
task_b_init_end:
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
    svc #0x108
    svc #0x100
    svc #0x102
    svc #0x104
    // EL0 compares the full-queue response to E_FULL=3 before echoing it.
    ldr x2, [x0, #72]
    cbz x2, 9f
    eor x3, x2, #3
    cbnz x3, el0_failure_b
    str x2, [x0, #64]
9:  ldr x2, [x0]
    mov x3, #0
    svc #0x106
    svc #0x101
    svc #0x107
    svc #0x110
    // Compare and acknowledge this task's denied save request (ID 5, code 4).
    ldr x2, [x0, #16]
    cbz x2, 13f
    sub x3, x2, #5
    cbnz x3, el0_failure_b
    str x2, [x0, #32]
    ldr x2, [x0, #24]
    eor x3, x2, #4
    cbnz x3, el0_failure_b
    str x2, [x0, #40]
13: ldr x2, [x0]
    mov x3, #0
    svc #0x111
    svc #0x103
    // Each task compares the delivered ID with its next expected ID and
    // the result with success=1 before echoing both to the kernel.
    ldr x2, [x0, #80]
    cbz x2, 10f
    ldr x3, [x0, #56]
    eor x3, x3, x2
    cbnz x3, el0_failure_b
    ldr x3, [x0, #112]
    eor x3, x3, #1
    cbnz x3, el0_failure_b
    str x2, [x0, #88]
    ldr x2, [x0, #112]
    str x2, [x0, #120]
    ldr x2, [x0, #56]
    add x2, x2, #1
    str x2, [x0, #56]
10: ldr x2, [x0]
    mov x3, #0
    svc #0x105
    svc #0x120
    svc #0x1ff
    add x19, x19, #1
    b task_b_loop
task_b_end:
el0_failure_b:
    svc #0x7bad
    b el0_failure_b
"#);

#[no_mangle]
pub extern "C" fn el0_finish()->! {
    check(unsafe{read_volatile(addr_of!(TICKS))}==8,"tick count");
    check(unsafe{read_volatile(addr_of!(CROSS_READ_DENIED))}==[true,true],"EL0 code isolation fault");
    let kb=physical(addr_of!(__stack_bottom) as usize);let kt=physical(addr_of!(__stack_top) as usize);
    let mut previous=[0u64;2];
    for i in 0..8 {
        let e=unsafe{read_volatile(addr_of!(EVENTS).cast::<Event>().add(i))};let actor=(i%2) as u64;
        let (lo,hi)=if actor==0 {(addr_of!(task_a_loop) as u64,addr_of!(task_a_end) as u64)} else {(addr_of!(task_b_loop) as u64,addr_of!(task_b_end) as u64)};
        let expected_spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(e.actor==actor && e.next==if i<7 {1-actor} else {2},"selected actor or next");
        // An IRQ can arrive while a task is in its constructor or syscall
        // path, before the next loop-counter increment. It must never go
        // backwards; each task must still progress across the whole probe.
        check(e.count>=previous[actor as usize],"task counter regressed");previous[actor as usize]=e.count;
        check(e.pc>=lo && e.pc<hi && e.pc%4==0,"user PC");
        check(e.spsr&0xf000_03ff==expected_spsr && e.sp as usize==super::mmu::ALIAS+PAGE,"user SPSR or SP");
        check(e.kernel_sp%16==0 && e.kernel_sp as usize>kb && (e.kernel_sp as usize)<=kt && (e.kernel_sp as usize).checked_sub(272).is_some_and(|n|n>=kb),"kernel IRQ stack");

    }
    for actor in 0..2usize {
        check(previous[actor]>0,"task did not progress during probe");
        let saved=unsafe{read_volatile(addr_of!(SAVED).cast::<u64>().add(actor))} as *const Frame;
        let expected=if actor==0 {addr_of!(A_FRAME)} else {addr_of!(B_FRAME)};
        check(saved==expected,"frame pointer");let f=unsafe{read_volatile(expected)};
        let spsr=if actor==0 {0xa000_0340} else {0x5000_0340};
        check(f.sp as usize==super::mmu::ALIAS+PAGE && f.spsr&0xf000_03ff==spsr,"final user frame");
        let marker=if actor==0 {0xa1a1} else {0xb2b2};
        let (init_lo,init_hi)=if actor==0 {(addr_of!(task_a_init) as u64,addr_of!(task_a_init_end) as u64)}else{(addr_of!(task_b_init) as u64,addr_of!(task_b_init_end) as u64)};
        let temp_ok=f.regs[3]==0 && ((f.elr>=init_lo && f.elr<init_hi) || f.regs[2]==marker || f.regs[2]==0);
        check(f.regs[0]==super::mmu::ALIAS as u64 && f.regs[1]==marker && temp_ok && f.regs[4]==1,"private data registers");
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
    check(unsafe{read_volatile(task_frame(0) as *const u64)}==0xa1a1 && unsafe{read_volatile(task_frame(1) as *const u64)}==0xb2b2,"private data markers");
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
