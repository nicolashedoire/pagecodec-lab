//! Narrow ELF64/AArch64 executable loader for the one-CPU QEMU guest.
//! Historical one-segment images plus a bounded static two-PT_LOAD subset.
//! No dynamic linker, interpreter or runtime relocations.
use alloc::vec::Vec;
use crate::mmu::USER_CODE;

const HEADER:usize=64;
const PHDR:usize=56;
const OFFSET:usize=HEADER+PHDR;
// A loaded image owns one RX frame. The ELF header and program header are
// not copied into that frame; only the validated code segment is copied.
const MAX_CODE:usize=4096;
const LEGACY_MAX_FILE:usize=OFFSET+MAX_CODE;
const MAX_FILE:usize=16384;
const USER_DATA:usize=0x5000_0000;

pub struct Data<'a> {pub bytes:&'a [u8],pub mem_size:usize}
pub struct Image<'a> { pub code:&'a [u8], pub entry_offset:usize, pub data:Option<Data<'a>> }
fn u16le(data:&[u8],i:usize)->Option<u16> {Some(u16::from_le_bytes(data.get(i..i.checked_add(2)?)?.try_into().ok()?))}
fn u32le(data:&[u8],i:usize)->Option<u32> {Some(u32::from_le_bytes(data.get(i..i.checked_add(4)?)?.try_into().ok()?))}
fn u64le(data:&[u8],i:usize)->Option<u64> {Some(u64::from_le_bytes(data.get(i..i.checked_add(8)?)?.try_into().ok()?))}

pub fn parse(data:&[u8])->Option<Image<'_>> {
 if data.len()<OFFSET || data.len()>MAX_FILE {return None}
 if u16le(data,56)?==2 {return parse_two(data)}
 if data.len()>LEGACY_MAX_FILE {return None}
 if data.get(..16)? != b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0" {return None}
 if u16le(data,16)?!=2 || u16le(data,18)?!=183 || u32le(data,20)?!=1
    || u64le(data,32)?!=HEADER as u64 || u64le(data,40)?!=0 || u32le(data,48)?!=0
    || u16le(data,52)?!=HEADER as u16 || u16le(data,54)?!=PHDR as u16
    || u16le(data,56)?!=1 || u16le(data,58)?!=0 || u16le(data,60)?!=0 || u16le(data,62)?!=0 {return None}
 let entry=u64le(data,24)?;
 if u32le(data,64)?!=1 || u32le(data,68)?!=5 || u64le(data,72)?!=OFFSET as u64
    || u64le(data,80)?!=USER_CODE as u64 || u64le(data,88)?!=0 || u64le(data,112)?!=4 {return None}
 let size=usize::try_from(u64le(data,96)?).ok()?;
 if size==0 || size>MAX_CODE || size%4!=0 || u64le(data,104)?!=size as u64 || OFFSET.checked_add(size)?!=data.len() {return None}
 let entry_offset=usize::try_from(entry.checked_sub(USER_CODE as u64)?).ok()?;
 if entry_offset%4!=0 || entry_offset.checked_add(4)?>size {return None}
 Some(Image{code:data.get(OFFSET..)?,entry_offset,data:None})
}

// Static executable subset, using the program headers of an actual linked ELF.
// Only pages managed by the guest are permitted; no interpreter or W+X.
fn parse_two(data:&[u8])->Option<Image<'_>> {
 if data.get(..7)?!=b"\x7fELF\x02\x01\x01" || !matches!(*data.get(7)?,0|3)
  || data.get(8..16)?!=[0u8;8] {return None}
 if u16le(data,16)?!=2 || u16le(data,18)?!=183 || u32le(data,20)?!=1 || u32le(data,48)?!=0
  || u64le(data,32)?!=HEADER as u64 || u16le(data,52)?!=HEADER as u16
  || u16le(data,54)?!=PHDR as u16 || u16le(data,56)?!=2 {return None}
 let ph_end=HEADER.checked_add(PHDR.checked_mul(2)?)?;
 if ph_end>data.len() {return None}
 // Section tables do not drive loading. If present, validate their bounds and
 // reject nonempty relocation sections rather than claiming relocation support.
 let shoff=usize::try_from(u64le(data,40)?).ok()?;let shsize=usize::from(u16le(data,58)?);let shcount=usize::from(u16le(data,60)?);
 if shoff==0 {if shsize!=0 || shcount!=0 || u16le(data,62)?!=0 {return None}}
 else {
  if shsize!=64 || shcount==0 || shoff.checked_add(shsize.checked_mul(shcount)?)?>data.len() {return None}
  let names=usize::from(u16le(data,62)?);if names!=0 && names>=shcount {return None}
  for i in 0..shcount {let at=shoff.checked_add(i.checked_mul(shsize)?)?;let kind=u32le(data,at+4)?;let len=u64le(data,at+32)?;if (kind==4 || kind==9) && len!=0 {return None}}
 }
 let mut code=None;let mut rw=None;
 for i in 0..2 {
  let at=HEADER.checked_add(i*PHDR)?;
  if u32le(data,at)?!=1 {return None}
  let flags=u32le(data,at+4)?;
  let off=usize::try_from(u64le(data,at+8)?).ok()?;
  let va=usize::try_from(u64le(data,at+16)?).ok()?;
  let filesz=usize::try_from(u64le(data,at+32)?).ok()?;
  let memsz=usize::try_from(u64le(data,at+40)?).ok()?;
  let align=usize::try_from(u64le(data,at+48)?).ok()?;
  if memsz==0 || memsz>MAX_CODE || filesz>memsz || (align>1 && (!align.is_power_of_two() || off%align!=va%align)) {return None}
  let end=off.checked_add(filesz)?;if end>data.len() || (filesz>0 && off<ph_end) {return None}
  let bytes=data.get(off..end)?;
  match (flags,va) {
   (5,USER_CODE) if code.is_none() && filesz>=4 && filesz==memsz=>code=Some((off,end,bytes)),
   (6,USER_DATA) if rw.is_none()=>rw=Some((off,end,bytes,memsz)),
   _=>return None,
  }
 }
 let (co,ce,code)=code?;let (ro,re,bytes,mem_size)=rw?;
 if ro<re && co<re && ro<ce {return None}
 let entry=usize::try_from(u64le(data,24)?).ok()?;
 let entry_offset=entry.checked_sub(USER_CODE)?;
 if entry_offset%4!=0 || entry_offset.checked_add(4)?>code.len() {return None}
 Some(Image{code,entry_offset,data:Some(Data{bytes,mem_size})})
}

/// Build demo files from two linked instruction sequences. This is not used by
/// `parse` to decide whether an arbitrary ELF is valid.
pub fn wrap(code:&[u8])->Option<Vec<u8>> {
 if code.is_empty() || code.len()>MAX_CODE || code.len()%4!=0 || OFFSET.checked_add(code.len())?>LEGACY_MAX_FILE {return None}
 let mut out=alloc::vec![0u8;OFFSET];
 out[..16].copy_from_slice(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0");
 out[16..18].copy_from_slice(&2u16.to_le_bytes());out[18..20].copy_from_slice(&183u16.to_le_bytes());
 out[20..24].copy_from_slice(&1u32.to_le_bytes());out[24..32].copy_from_slice(&(USER_CODE as u64).to_le_bytes());
 out[32..40].copy_from_slice(&(HEADER as u64).to_le_bytes());out[52..54].copy_from_slice(&(HEADER as u16).to_le_bytes());
 out[54..56].copy_from_slice(&(PHDR as u16).to_le_bytes());out[56..58].copy_from_slice(&1u16.to_le_bytes());
 out[64..68].copy_from_slice(&1u32.to_le_bytes());out[68..72].copy_from_slice(&5u32.to_le_bytes());
 out[72..80].copy_from_slice(&(OFFSET as u64).to_le_bytes());out[80..88].copy_from_slice(&(USER_CODE as u64).to_le_bytes());
 out[96..104].copy_from_slice(&(code.len() as u64).to_le_bytes());out[104..112].copy_from_slice(&(code.len() as u64).to_le_bytes());
 out[112..120].copy_from_slice(&4u64.to_le_bytes());out.extend_from_slice(code);Some(out)
}
