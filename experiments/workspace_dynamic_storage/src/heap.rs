//! Reclaiming heap for the single-CPU VM service. DMA is a separate pool.
//!
//! Metadata is fixed and never allocated from this heap. Interrupts are
//! masked while manipulating the ordered extent table. This is not SMP-safe.
use core::{alloc::{GlobalAlloc,Layout},arch::asm,cell::UnsafeCell,ptr::{addr_of_mut,null_mut},sync::atomic::{AtomicBool,AtomicUsize,Ordering}};
use serde_json::{json,Value};

const MAX_EXTENTS:usize=4096;
#[derive(Clone,Copy)]
struct Extent {start:usize,len:usize,used:bool}
const EMPTY:Extent=Extent{start:0,len:0,used:false};
struct State {extents:[Extent;MAX_EXTENTS],count:usize,initialized:bool,live:usize,peak:usize,successes:usize,frees:usize,active:usize,metadata_exhaustions:usize}
impl State {
 const fn new()->Self {Self{extents:[EMPTY;MAX_EXTENTS],count:0,initialized:false,live:0,peak:0,successes:0,frees:0,active:0,metadata_exhaustions:0}}
 fn init(&mut self) {if !self.initialized {self.extents[0]=Extent{start:0,len:crate::HEAP_SIZE,used:false};self.count=1;self.initialized=true}}
 fn allocate(&mut self,base:usize,layout:Layout)->*mut u8 {
  self.init();let size=layout.size().max(1);let mask=layout.align()-1;
  for i in 0..self.count {
   let block=self.extents[i];if block.used {continue}
   let Some(start)=base.checked_add(block.start).and_then(|v|v.checked_add(mask)).map(|v|v&!mask).and_then(|v|v.checked_sub(base)) else {continue};
   let Some(end)=start.checked_add(size) else {continue};let Some(limit)=block.start.checked_add(block.len) else {continue};
   if start<block.start || end>limit {continue}
   let prefix=start-block.start;let suffix=limit-end;let extra=usize::from(prefix!=0)+usize::from(suffix!=0);
   if self.count.checked_add(extra).is_none_or(|n|n>MAX_EXTENTS) {self.metadata_exhaustions=self.metadata_exhaustions.saturating_add(1);continue}
   for j in (i+1..self.count).rev() {self.extents[j+extra]=self.extents[j]}
   let mut at=i;
   if prefix!=0 {self.extents[at]=Extent{start:block.start,len:prefix,used:false};at+=1}
   self.extents[at]=Extent{start,len:size,used:true};at+=1;
   if suffix!=0 {self.extents[at]=Extent{start:end,len:suffix,used:false}}
   self.count+=extra;self.live+=size;self.peak=self.peak.max(self.live);self.active+=1;self.successes=self.successes.saturating_add(1);
   return (base+start) as *mut u8
  }
  null_mut()
 }
 fn remove(&mut self,index:usize) {for i in index+1..self.count {self.extents[i-1]=self.extents[i]};self.count-=1;self.extents[self.count]=EMPTY}
 fn release(&mut self,base:usize,ptr:*mut u8,layout:Layout)->bool {
  let Some(start)=(ptr as usize).checked_sub(base) else{return false};let size=layout.size().max(1);
  let Some(mut i)=(0..self.count).find(|&j|self.extents[j].used && self.extents[j].start==start && self.extents[j].len==size) else{return false};
  self.extents[i].used=false;self.live-=size;self.active-=1;self.frees=self.frees.saturating_add(1);
  if i>0 && !self.extents[i-1].used && self.extents[i-1].start.checked_add(self.extents[i-1].len)==Some(self.extents[i].start) {
   self.extents[i-1].len+=self.extents[i].len;self.remove(i);i-=1;
  }
  if i+1<self.count && !self.extents[i+1].used && self.extents[i].start.checked_add(self.extents[i].len)==Some(self.extents[i+1].start) {
   self.extents[i].len+=self.extents[i+1].len;self.remove(i+1);
  }
  true
 }
 fn largest(&self)->usize {self.extents[..self.count].iter().filter(|b|!b.used).map(|b|b.len).max().unwrap_or(0)}
}

static ATTEMPTS:AtomicUsize=AtomicUsize::new(0);
static LOCKED:AtomicBool=AtomicBool::new(false);
pub struct Heap(UnsafeCell<State>);
unsafe impl Sync for Heap {}
impl Heap {pub const fn new()->Self {Self(UnsafeCell::new(State::new()))}}
struct Guard {daif:u64}
impl Guard {
 fn acquire()->Option<Self> {
  let daif:u64;unsafe{asm!("mrs {}, DAIF","msr daifset, #0xf",out(reg) daif,options(nostack))};
  if LOCKED.swap(true,Ordering::Acquire) {unsafe{asm!("msr DAIF, {}",in(reg) daif,options(nostack))};None}else{Some(Self{daif})}
 }
}
impl Drop for Guard {fn drop(&mut self) {LOCKED.store(false,Ordering::Release);unsafe{asm!("msr DAIF, {}",in(reg) self.daif,options(nostack))}}}
fn base()->usize {addr_of_mut!(crate::HEAP) as usize}
unsafe impl GlobalAlloc for Heap {
 unsafe fn alloc(&self,layout:Layout)->*mut u8 {
  ATTEMPTS.fetch_add(1,Ordering::Relaxed);
  let Some(_guard)=Guard::acquire() else{return null_mut()};
  (*self.0.get()).allocate(base(),layout)
 }
 unsafe fn dealloc(&self,ptr:*mut u8,layout:Layout) {
  let Some(_guard)=Guard::acquire() else{crate::halt()};
  if !(*self.0.get()).release(base(),ptr,layout) {crate::halt()}
 }
}
pub fn allocation_attempts()->usize {ATTEMPTS.load(Ordering::Relaxed)}
#[derive(Clone,Copy)]
struct Snapshot {live:usize,peak:usize,largest:usize,successes:usize,frees:usize,active:usize,metadata_exhaustions:usize,attempts:usize}
fn snapshot()->Snapshot {
 let Some(_guard)=Guard::acquire() else{crate::halt()};
 let s=unsafe{&mut *crate::ALLOC.0.get()};s.init();
 Snapshot{live:s.live,peak:s.peak,largest:s.largest(),successes:s.successes,frees:s.frees,active:s.active,metadata_exhaustions:s.metadata_exhaustions,attempts:allocation_attempts()}
}
pub fn stats()->Value {
 let s=snapshot();json!({"capacity":crate::HEAP_SIZE,"live_bytes":s.live,"peak_bytes":s.peak,"free_bytes":crate::HEAP_SIZE-s.live,"largest_free":s.largest,"alloc_attempts":s.attempts,"alloc_successes":s.successes,"frees":s.frees,"active_allocations":s.active,"metadata_exhaustions":s.metadata_exhaustions})
}
/// Direct, bounded allocator exercise. Never exhausts the service's heap.
/// OOM here does not imply that infallible JSON/BTreeMap OOM is recoverable.
pub fn exercise()->Value {
 let before=snapshot();let block=Layout::from_size_align(16*1024,16).unwrap();
 let large=Layout::from_size_align(32*1024,16).unwrap();
 let huge=Layout::from_size_align(crate::HEAP_SIZE+1,16).unwrap();
 let small=Layout::from_size_align(32,32).unwrap();
 let (a,b,c,oom)=unsafe{(crate::ALLOC.alloc(block),crate::ALLOC.alloc(block),crate::ALLOC.alloc(block),crate::ALLOC.alloc(huge))};
 let contiguous=!a.is_null() && !b.is_null() && !c.is_null() && a as usize+block.size()==b as usize && b as usize+block.size()==c as usize;
 let mut coalesced=false;
 unsafe {
  if !a.is_null() {crate::ALLOC.dealloc(a,block)}
  if !b.is_null() {crate::ALLOC.dealloc(b,block)}
  if contiguous {let joined=crate::ALLOC.alloc(large);coalesced=joined==a;if !joined.is_null(){crate::ALLOC.dealloc(joined,large)}}
  if !c.is_null() {crate::ALLOC.dealloc(c,block)}
 }
 let recovered_ptr=unsafe{crate::ALLOC.alloc(small)};let recovered=!recovered_ptr.is_null() && recovered_ptr as usize%small.align()==0;
 if !recovered_ptr.is_null() {unsafe{crate::ALLOC.dealloc(recovered_ptr,small)}}
 // Exercise small and page-aligned layouts while all three are live; validate
 // boundaries and free in a different order to exercise both merge directions.
 let layouts=[Layout::from_size_align(1,1).unwrap(),Layout::from_size_align(33,16).unwrap(),Layout::from_size_align(123,4096).unwrap()];
 let mut ptrs=[null_mut();3];let mut alignments_ok=true;
 for i in 0..3 {ptrs[i]=unsafe{crate::ALLOC.alloc(layouts[i])};if ptrs[i].is_null() || ptrs[i] as usize%layouts[i].align()!=0 {alignments_ok=false}}
 for i in 0..3 {if !ptrs[i].is_null() {unsafe{ptrs[i].write_volatile(0x31+i as u8);ptrs[i].add(layouts[i].size()-1).write_volatile(0x31+i as u8)}}}
 for i in 0..3 {if !ptrs[i].is_null() {unsafe{alignments_ok &= ptrs[i].read_volatile()==0x31+i as u8 && ptrs[i].add(layouts[i].size()-1).read_volatile()==0x31+i as u8}}}
 for i in [1usize,0,2] {if !ptrs[i].is_null() {unsafe{crate::ALLOC.dealloc(ptrs[i],layouts[i])}}}
 let after=snapshot();let restored=after.live==before.live && after.active==before.active && after.largest==before.largest;
 json!({"coalesced":coalesced,"oom_reported":oom.is_null(),"recovered":recovered,"alignments_ok":alignments_ok,"live_restored":restored})
}
