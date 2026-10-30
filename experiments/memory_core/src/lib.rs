//! Single-pool physical frame state model; no metadata allocation or MMU setup.
#![no_std]
#![forbid(unsafe_code)]

/// This demonstrator chooses a fixed 4 KiB frame, not the native Mac page size.
pub const FRAME_SIZE: usize = 4096;
/// Fixed metadata capacity; a production allocator needs scalable boot-time metadata.
pub const MAX_FRAMES: usize = 2048;
const WORD_BITS: usize = 64;
const WORDS: usize = MAX_FRAMES / WORD_BITS;

/// Half-open byte interval. Both endpoints must be frame aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservedRange { pub start: usize, pub end: usize }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    UnalignedBase,
    InvalidFrameCount,
    AddressOverflow,
    InvalidReservation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeError { Unaligned, OutsidePool, Reserved, NotAllocated }

/// Records allocation and reservation state only; returned addresses are not mapped,
/// zeroed, or safe to dereference without a trustworthy platform memory map.
pub struct FrameAllocator {
    base: usize,
    end: usize,
    count: usize,
    unavailable: [u64; WORDS],
    allocated: [u64; WORDS],
}

impl FrameAllocator {
    /// Construct a contiguous authorized pool. Every reserved range must be aligned,
    /// nonempty and fully inside that pool. Overlapping reservations are allowed.
    pub fn new(base: usize, count: usize, reservations: &[ReservedRange]) -> Result<Self, InitError> {
        if base % FRAME_SIZE != 0 { return Err(InitError::UnalignedBase); }
        if count == 0 || count > MAX_FRAMES { return Err(InitError::InvalidFrameCount); }
        let size = count.checked_mul(FRAME_SIZE).ok_or(InitError::AddressOverflow)?;
        let end = base.checked_add(size).ok_or(InitError::AddressOverflow)?;
        let mut this = Self { base, end, count, unavailable: [0; WORDS], allocated: [0; WORDS] };
        for range in reservations {
            if range.start >= range.end || range.start % FRAME_SIZE != 0
                || range.end % FRAME_SIZE != 0 || range.start < base || range.end > end {
                return Err(InitError::InvalidReservation);
            }
            let first = (range.start - base) / FRAME_SIZE;
            let after_last = (range.end - base) / FRAME_SIZE;
            for i in first..after_last { set(&mut this.unavailable, i); }
        }
        Ok(this)
    }

    /// Return the lowest available frame address, or `None` when the pool is exhausted.
    pub fn allocate(&mut self) -> Option<usize> {
        for i in 0..self.count {
            if !get(&self.unavailable, i) && !get(&self.allocated, i) {
                set(&mut self.allocated, i);
                // `new` verified pool-end arithmetic and i is within that pool.
                return Some(self.base + i * FRAME_SIZE);
            }
        }
        None
    }

    /// Only an aligned, allocated, non-reserved address in the pool can be freed.
    pub fn free(&mut self, address: usize) -> Result<(), FreeError> {
        if address % FRAME_SIZE != 0 { return Err(FreeError::Unaligned); }
        if address < self.base || address >= self.end { return Err(FreeError::OutsidePool); }
        let i = (address - self.base) / FRAME_SIZE;
        if get(&self.unavailable, i) { return Err(FreeError::Reserved); }
        if !get(&self.allocated, i) { return Err(FreeError::NotAllocated); }
        self.allocated[i / WORD_BITS] &= !(1u64 << (i % WORD_BITS));
        Ok(())
    }
}

fn get(bitmap: &[u64; WORDS], index: usize) -> bool {
    bitmap[index / WORD_BITS] & (1u64 << (index % WORD_BITS)) != 0
}
fn set(bitmap: &mut [u64; WORDS], index: usize) {
    bitmap[index / WORD_BITS] |= 1u64 << (index % WORD_BITS);
}
