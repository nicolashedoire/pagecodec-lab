//! Two-copy tree image store for an explicitly provisioned QEMU disk.
//!
//! Reserved LBAs are [256,512) and [512,768), not the existing workspace or
//! program snapshots. This is not a general disk partitioning scheme. Calling
//! `store` is permitted only after the owner explicitly establishes that the
//! QEMU disk and those regions are dedicated to this format. A magic marker or
//! blank bytes cannot establish ownership of arbitrary pre-existing sectors.
//!
//! One service owns all disk operations on one CPU. `STORAGE_WRITE_BLOCKED`
//! is shared with the existing writers. Stage, flush, commit, flush, readback
//! detects some errors but is not a promise of power-loss durability: VirtIO
//! flush may be unavailable/no-op, and disk operations may block indefinitely.
//! The FNV checksum detects some accidental corruption, not malicious changes.
//! This tree volume is independent of the existing document snapshot. There
//! is no shared transaction, automatic migration, or document synchronization.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use virtio_drivers::{device::blk::VirtIOBlk, transport::Transport};
use crate::{tree_store, VmHal, STORAGE_WRITE_BLOCKED};

const SECTOR_BYTES: usize = 512;
const SLOT_SECTORS: usize = 256;
const RECORD_BYTES: usize = SECTOR_BYTES * SLOT_SECTORS;
const SLOT_START: [usize; 2] = [256, 512];
const HEADER: usize = SECTOR_BYTES;
const MAGIC: &[u8; 8] = b"TREE0001";
const COMMITTED: u8 = 0xa5;
const MAX_IMAGE: usize = 12 + tree_store::MAX_ENTRIES * (8 + tree_store::MAX_PATH) + tree_store::MAX_CONTENT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState { Blank, Unknown, Invalid, Valid }

pub struct Status {
    pub slots: [SlotState; 2],
    pub selected: Option<usize>,
    pub generation: Option<u64>,
    pub selection_error: bool,
    pub write_blocked: bool,
}

enum Slot<'a> { Blank, Unknown, Invalid, Valid(u64, &'a [u8]) }
impl Slot<'_> {
    fn state(&self) -> SlotState {
        match self { Self::Blank => SlotState::Blank, Self::Unknown => SlotState::Unknown, Self::Invalid => SlotState::Invalid, Self::Valid(..) => SlotState::Valid }
    }
}

fn buffer() -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(RECORD_BYTES).map_err(|_| "tree buffer allocation failed")?;
    bytes.resize(RECORD_BYTES, 0);
    Ok(bytes)
}

// The checksum includes generation, image length, and image; commit and
// reserved/padding bytes are separately checked and are never authenticated.
fn checksum(data: &[u8], image_len: usize) -> u64 {
    data[8..20].iter().chain(data[HEADER..HEADER + image_len].iter())
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}

fn inspect(data: &[u8]) -> Result<Slot<'_>, &'static str> {
    if data.len() != RECORD_BYTES { return Ok(Slot::Unknown); }
    if data.iter().all(|b| *b == 0) { return Ok(Slot::Blank); }
    if &data[..8] != MAGIC { return Ok(Slot::Unknown); }
    let generation = u64::from_le_bytes(data[8..16].try_into().map_err(|_| "tree header invalid")?);
    let len = u32::from_le_bytes(data[16..20].try_into().map_err(|_| "tree header invalid")?) as usize;
    let sum = u64::from_le_bytes(data[20..28].try_into().map_err(|_| "tree header invalid")?);
    if generation == 0 || len == 0 || len > MAX_IMAGE || HEADER.checked_add(len).is_none_or(|end| end > RECORD_BYTES)
        || data[28] != COMMITTED || data[29..HEADER].iter().any(|b| *b != 0) {
        return Ok(Slot::Invalid);
    }
    let end = HEADER + len;
    if data[end..].iter().any(|b| *b != 0) || checksum(data, len) != sum { return Ok(Slot::Invalid); }
    match tree_store::TreeStore::decode(&data[HEADER..end]) {
        Ok(_) => Ok(Slot::Valid(generation, &data[HEADER..end])),
        Err(tree_store::Error::OutOfMemory) => Err("tree validation allocation failed"),
        Err(_) => Ok(Slot::Invalid),
    }
}

fn read_slots<T: Transport>(disk: &mut VirtIOBlk<VmHal,T>) -> Result<(Vec<u8>,Vec<u8>), &'static str> {
    // Match the explicitly provisioned demo disk. Capacity alone is not proof
    // that the disk is dedicated; callers must establish that separately.
    if disk.capacity() != 2048 { return Err("unexpected capacity"); }
    let mut a = buffer()?;
    let mut b = buffer()?;
    disk.read_blocks(SLOT_START[0], &mut a).map_err(|_| "tree slot read failed")?;
    disk.read_blocks(SLOT_START[1], &mut b).map_err(|_| "tree slot read failed")?;
    Ok((a,b))
}

fn select<'a>(a: &Slot<'a>, b: &Slot<'a>) -> Result<Option<(usize,u64,&'a [u8])>, &'static str> {
    match (a,b) {
        (Slot::Valid(ga,ia), Slot::Valid(gb,ib)) => {
            if ga == gb { Err("duplicate tree generations") }
            else if ga > gb { Ok(Some((0,*ga,ia))) } else { Ok(Some((1,*gb,ib))) }
        }
        (Slot::Valid(g,image), _) => Ok(Some((0,*g,image))),
        (_, Slot::Valid(g,image)) => Ok(Some((1,*g,image))),
        (Slot::Blank, Slot::Blank) => Ok(None),
        _ => Err("no valid committed tree"),
    }
}

pub fn status<T: Transport>(disk: &mut VirtIOBlk<VmHal,T>) -> Result<Status, &'static str> {
    let (a,b) = read_slots(disk)?;
    let (a,b) = (inspect(&a)?, inspect(&b)?);
    let (selected, generation, selection_error) = match select(&a,&b) {
        Ok(Some((slot,generation,_))) => (Some(slot),Some(generation),false),
        Ok(None) => (None,None,false),
        Err(_) => (None,None,true),
    };
    Ok(Status { slots: [a.state(),b.state()], selected, generation, selection_error,
        write_blocked: STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) })
}

/// Read-only recovery may use a valid copy even beside an invalid/unknown
/// region. It never repairs either region and does not imply safe overwriting.
pub fn load<T: Transport>(disk: &mut VirtIOBlk<VmHal,T>) -> Result<tree_store::TreeStore, &'static str> {
    let (raw_a,raw_b) = read_slots(disk)?;
    let (a,b) = (inspect(&raw_a)?,inspect(&raw_b)?);
    let (_,_,image) = select(&a,&b)?.ok_or("tree not found")?;
    tree_store::TreeStore::decode(image).map_err(|error| match error {
        tree_store::Error::OutOfMemory => "tree load allocation failed",
        _ => "tree image invalid",
    })
}

/// `dedicated` must reflect the caller's explicit provisioning decision, not
/// merely a capacity/magic check. No format or repair is performed implicitly.
pub fn store<T: Transport>(disk: &mut VirtIOBlk<VmHal,T>, tree: &tree_store::TreeStore, dedicated: bool) -> Result<(usize,u64), &'static str> {
    if !dedicated { return Err("tree disk not explicitly provisioned"); }
    if STORAGE_WRITE_BLOCKED.load(Ordering::Relaxed) { return Err("storage write blocked until restart"); }
    let image = tree.encode().map_err(|error| match error {
        tree_store::Error::OutOfMemory => "tree image allocation failed",
        _ => "tree image invalid",
    })?;
    if image.is_empty() || image.len() > MAX_IMAGE { return Err("tree image too large"); }
    tree_store::TreeStore::decode(&image).map_err(|error| match error {
        tree_store::Error::OutOfMemory => "tree validation allocation failed",
        _ => "tree image invalid",
    })?;
    let (raw_a,raw_b) = read_slots(disk)?;
    let (a,b) = (inspect(&raw_a)?,inspect(&raw_b)?);
    if matches!(a,Slot::Unknown) || matches!(b,Slot::Unknown) { return Err("unknown tree region"); }
    if matches!(a,Slot::Invalid) || matches!(b,Slot::Invalid) { return Err("tree region needs repair"); }
    let (slot,generation) = match select(&a,&b)? {
        None => (0,1),
        Some((i,g,_)) => (1-i,g.checked_add(1).ok_or("tree generation exhausted")?),
    };
    // Neither input record is needed while constructing or verifying output.
    drop(raw_a);
    drop(raw_b);
    let mut record = buffer()?;
    let mut verified = buffer()?;
    record[..8].copy_from_slice(MAGIC);
    record[8..16].copy_from_slice(&generation.to_le_bytes());
    record[16..20].copy_from_slice(&(image.len() as u32).to_le_bytes());
    record[HEADER..HEADER+image.len()].copy_from_slice(&image);
    let sum = checksum(&record,image.len());
    record[20..28].copy_from_slice(&sum.to_le_bytes());
    // After this point *any* failure keeps the shared block asserted. Buffers
    // are allocated before the first disk mutation. No EL0 success yet.
    STORAGE_WRITE_BLOCKED.compare_exchange(false,true,Ordering::AcqRel,Ordering::Relaxed)
        .map_err(|_| "storage write blocked until restart")?;
    disk.write_blocks(SLOT_START[slot],&record).map_err(|_| "tree stage write failed")?;
    disk.flush().map_err(|_| "tree stage flush failed")?;
    record[28] = COMMITTED;
    disk.write_blocks(SLOT_START[slot],&record[..HEADER]).map_err(|_| "tree commit write failed")?;
    disk.flush().map_err(|_| "tree commit flush failed")?;
    disk.read_blocks(SLOT_START[slot],&mut verified).map_err(|_| "tree readback failed")?;
    if verified != record || !matches!(inspect(&verified)?,Slot::Valid(g,_) if g==generation) {
        return Err("tree readback mismatch");
    }
    STORAGE_WRITE_BLOCKED.store(false,Ordering::Release);
    Ok((slot,generation))
}
