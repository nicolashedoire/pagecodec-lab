//! Narrow ELF64/AArch64 executable loader for the one-CPU QEMU guest.
//! One fully file-backed RX segment only: no dynamic linker or relocation.
use alloc::vec::Vec;
use crate::mmu::USER_CODE;

const HEADER:usize=64;
const PHDR:usize=56;
const OFFSET:usize=HEADER+PHDR;
const MAX_FILE:usize=512;

pub struct Image<'a> { pub code:&'a [u8], pub entry_offset:usize }
fn u16le(data:&[u8],i:usize)->Option<u16> {Some(u16::from_le_bytes(data.get(i..i.checked_add(2)?)?.try_into().ok()?))}
fn u32le(data:&[u8],i:usize)->Option<u32> {Some(u32::from_le_bytes(data.get(i..i.checked_add(4)?)?.try_into().ok()?))}
fn u64le(data:&[u8],i:usize)->Option<u64> {Some(u64::from_le_bytes(data.get(i..i.checked_add(8)?)?.try_into().ok()?))}

pub fn parse(data:&[u8])->Option<Image<'_>> {
 if data.len()<OFFSET || data.len()>MAX_FILE {return None}
 if data.get(..16)? != b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0" {return None}
 if u16le(data,16)?!=2 || u16le(data,18)?!=183 || u32le(data,20)?!=1
    || u64le(data,32)?!=HEADER as u64 || u64le(data,40)?!=0 || u32le(data,48)?!=0
    || u16le(data,52)?!=HEADER as u16 || u16le(data,54)?!=PHDR as u16
    || u16le(data,56)?!=1 || u16le(data,58)?!=0 || u16le(data,60)?!=0 || u16le(data,62)?!=0 {return None}
 let entry=u64le(data,24)?;
 if u32le(data,64)?!=1 || u32le(data,68)?!=5 || u64le(data,72)?!=OFFSET as u64
    || u64le(data,80)?!=USER_CODE as u64 || u64le(data,88)?!=0 || u64le(data,112)?!=4 {return None}
 let size=usize::try_from(u64le(data,96)?).ok()?;
 if size==0 || size%4!=0 || u64le(data,104)?!=size as u64 || OFFSET.checked_add(size)?!=data.len() {return None}
 let entry_offset=usize::try_from(entry.checked_sub(USER_CODE as u64)?).ok()?;
 if entry_offset%4!=0 || entry_offset.checked_add(4)?>size {return None}
 Some(Image{code:data.get(OFFSET..)?,entry_offset})
}

/// Build demo files from two linked instruction sequences. This is not used by
/// `parse` to decide whether an arbitrary ELF is valid.
pub fn wrap(code:&[u8])->Option<Vec<u8>> {
 if code.is_empty() || code.len()%4!=0 || OFFSET.checked_add(code.len())?>MAX_FILE {return None}
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
