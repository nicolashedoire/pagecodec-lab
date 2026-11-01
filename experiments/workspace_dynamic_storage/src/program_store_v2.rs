//! Version 2 program area on an explicitly dedicated QEMU virtual disk.
//! Two 33-sector records at sectors 64..130; version 1 remains untouched.
use alloc::vec::Vec;
use serde_json::{json,Value};
use core::sync::atomic::Ordering;
use virtio_drivers::{device::blk::VirtIOBlk,transport::Transport};
use crate::{VmHal,STORAGE_WRITE_BLOCKED};

const RECORD:usize=16896;
const HEADER:usize=512;
const IMAGE_MAX:usize=16384;
const SECTORS:[usize;2]=[64,97];
const MAGIC:&[u8;8]=b"PROG0002";

enum Slot<'a> {Blank,Unknown,Invalid,Valid(u64,&'a [u8])}
fn hash(record:&[u8],len:usize)->u64 {
    record[8..20].iter().chain(record[HEADER..HEADER+len].iter()).fold(0xcbf29ce484222325u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100000001b3))
}
fn decode(data:&[u8])->Slot<'_> {
    if data.len()!=RECORD {return Slot::Unknown}
    if data.iter().all(|b|*b==0) {return Slot::Blank}
    if &data[..8]!=MAGIC {return Slot::Unknown}
    let generation=u64::from_le_bytes(data[8..16].try_into().unwrap());
    let len=u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
    // A magic prefix alone is not sufficient authority to overwrite a region.
    if generation==0 || len==0 || len>IMAGE_MAX || !matches!(data[28],0|0xa5) || data[29..HEADER].iter().any(|b|*b!=0) {return Slot::Unknown}
    let sum=u64::from_le_bytes(data[20..28].try_into().unwrap());
    if data[28]!=0xa5 || data[HEADER+len..].iter().any(|b|*b!=0) || hash(data,len)!=sum || crate::loader::parse(&data[HEADER..HEADER+len]).is_none() {return Slot::Invalid}
    Slot::Valid(generation,&data[HEADER..HEADER+len])
}
fn buffer()->Result<Vec<u8>,&'static str> {
    let mut bytes=Vec::new();bytes.try_reserve_exact(RECORD).map_err(|_|"program v2 buffer allocation failed")?;bytes.resize(RECORD,0);Ok(bytes)
}
fn read<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>)->Result<(Vec<u8>,Vec<u8>),&'static str> {
    if disk.capacity()!=2048 {return Err("unexpected capacity")}
    let mut a=buffer()?;let mut b=buffer()?;
    disk.read_blocks(SECTORS[0],&mut a).map_err(|_|"program slot read failed")?;
    disk.read_blocks(SECTORS[1],&mut b).map_err(|_|"program slot read failed")?;
    Ok((a,b))
}
fn label(s:&Slot<'_>)->&'static str {match s {Slot::Blank=>"blank",Slot::Unknown=>"unknown",Slot::Invalid=>"invalid",Slot::Valid(..)=>"valid"}}
fn select<'a>(a:&Slot<'a>,b:&Slot<'a>)->Result<Option<(usize,u64,&'a [u8])>,&'static str> {
    match (a,b) {
        (Slot::Valid(ga,ia),Slot::Valid(gb,ib))=>if ga==gb {Err("duplicate program generations")}else if ga>gb {Ok(Some((0,*ga,ia)))}else{Ok(Some((1,*gb,ib)))},
        (Slot::Valid(g,i),_)=>Ok(Some((0,*g,i))),
        (_,Slot::Valid(g,i))=>Ok(Some((1,*g,i))),
        (Slot::Blank,Slot::Blank)=>Ok(None),
        _=>Err("no valid committed program"),
    }
}
pub fn status<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>)->Result<Value,&'static str> {
    let (a,b)=read(disk)?;let (a,b)=(decode(&a),decode(&b));
    let selection=select(&a,&b);
    let (state,generation)=match selection {Ok(Some((_,g,_)))=>("valid",Some(g)),Ok(None)=>("blank",None),Err(_)=>("invalid",None)};
    Ok(json!({"state":state,"slots":[label(&a),label(&b)],"generation":generation,"write_blocked":STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed),"unknown_region":matches!(a,Slot::Unknown)||matches!(b,Slot::Unknown)}))
}
pub fn load<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>)->Result<Vec<u8>,&'static str> {
    let (a,b)=read(disk)?;let (a,b)=(decode(&a),decode(&b));
    let (_,_,image)=select(&a,&b)?.ok_or("program not found")?;
    let mut copy=Vec::new();copy.try_reserve_exact(image.len()).map_err(|_|"program v2 buffer allocation failed")?;
    copy.extend_from_slice(image);Ok(copy)
}
pub fn store<T:Transport>(disk:&mut VirtIOBlk<VmHal,T>,image:&[u8])->Result<(usize,u64),&'static str> {
    if STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) {return Err("storage write blocked until restart")}
    if image.len()>IMAGE_MAX || crate::loader::parse(image).is_none() {return Err("invalid staged ELF")}
    let (a,b)=read(disk)?;let (a,b)=(decode(&a),decode(&b));
    if matches!(a,Slot::Unknown) || matches!(b,Slot::Unknown) {return Err("unknown program region")}
    // A damaged or interrupted record is never silently reformatted, even
    // if its neighbor is readable. Recovery needs a separate explicit policy.
    if matches!(a,Slot::Invalid) || matches!(b,Slot::Invalid) {return Err("program region needs repair")}
    let (slot,generation)=match select(&a,&b)? {None=>(0,1),Some((i,g,_))=>(1-i,g.checked_add(1).ok_or("program generation exhausted")?)};
    let mut record=buffer()?;record[..8].copy_from_slice(MAGIC);
    record[8..16].copy_from_slice(&generation.to_le_bytes());record[16..20].copy_from_slice(&(image.len() as u32).to_le_bytes());
    record[HEADER..HEADER+image.len()].copy_from_slice(image);let checksum=hash(&record,image.len());record[20..28].copy_from_slice(&checksum.to_le_bytes());
    // No other task can mutate storage during this single-CPU service call.
    // Once a write begins, any error leaves the shared disk state uncertain.
    STORAGE_WRITE_BLOCKED.store(true,Ordering::Relaxed);
    disk.write_blocks(SECTORS[slot],&record).map_err(|_|"program stage write failed")?;
    disk.flush().map_err(|_|"program stage flush failed")?;
    record[28]=0xa5;
    disk.write_blocks(SECTORS[slot],&record[..HEADER]).map_err(|_|"program commit write failed")?;
    disk.flush().map_err(|_|"program commit flush failed")?;
    #[cfg(feature="storage-fault-test")]
    if crate::FAIL_NEXT_PROGRAM_V2_POSTCOMMIT.swap(false,Ordering::Relaxed) {return Err("injected program v2 postcommit confirmation failure")}
    let mut verified=buffer()?;disk.read_blocks(SECTORS[slot],&mut verified).map_err(|_|"program readback failed")?;
    if verified!=record || !matches!(decode(&verified),Slot::Valid(g,_) if g==generation) {return Err("program readback mismatch")}
    STORAGE_WRITE_BLOCKED.store(false,Ordering::Relaxed);
    Ok((slot,generation))
}
