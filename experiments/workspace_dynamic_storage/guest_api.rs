//! Minimal Rust interface for independent guest applications.
//!
//! This ABI is specific to this AArch64 guest; it is not POSIX. The kernel
//! currently supplies up to eight private 4 KiB heap pages per process.
//! File names and buffers must reside in guest data, stack or heap pages,
//! not in the read-only code page. File methods poll without a deadline and
//! can wait indefinitely if the service stops making progress.
use core::{arch::asm, ptr::NonNull, slice};

pub const PAGE_SIZE: usize = 4096;
pub const PAGE_LIMIT: usize = 8;
const USER_HEAP: usize = 0x5000_4000;
const EAGAIN: i64 = -11;

/// Owns the caller's private page. Dropping a page attempts to return it.
/// If the kernel reports it busy, the process retains it until it exits.
#[must_use]
pub struct HeapPage { ptr: NonNull<u8>, index: usize, active: bool }

impl HeapPage {
    pub fn allocate() -> Result<Self, i64> {
        let mut result: u64;
        // The kernel returns a page address or a negative error in x0.
        unsafe { asm!("svc #0x180", lateout("x0") result, options(nostack)); }
        let result = result as i64;
        if result < 0 { return Err(result); }
        if result as usize != USER_HEAP { return Err(-5); }
        let ptr = NonNull::new(result as usize as *mut u8).ok_or(-5)?;
        Ok(Self { ptr, index: 0, active: true })
    }

    /// Allocate one of the eight independently owned page slots.
    pub fn allocate_index(index: usize) -> Result<Self, i64> {
        if index >= PAGE_LIMIT { return Err(-22); }
        let mut result = index as u64;
        unsafe { asm!("svc #0x182", inout("x0") result, options(nostack)); }
        let result = result as i64;
        if result < 0 { return Err(result); }
        let expected = USER_HEAP + index * PAGE_SIZE;
        if result as usize != expected { return Err(-5); }
        let ptr = NonNull::new(expected as *mut u8).ok_or(-5)?;
        Ok(Self { ptr, index, active: true })
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // The kernel owns the mapping lifecycle; this unique owner borrows it.
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), PAGE_SIZE) }
    }

    /// Return the page, retaining ownership on failure (for example, -16 busy).
    pub fn release(mut self) -> Result<(), (i64, Self)> {
        let result = self.free_raw();
        if result == 0 { self.active = false; Ok(()) } else { Err((result, self)) }
    }

    fn free_raw(&self) -> i64 {
        let mut result = if self.index == 0 { self.ptr.as_ptr() as u64 } else { self.index as u64 };
        if self.index == 0 {
            unsafe { asm!("svc #0x181", inout("x0") result, options(nostack)); }
        } else {
            unsafe { asm!("svc #0x183", inout("x0") result, options(nostack)); }
        }
        result as i64
    }
}

impl Drop for HeapPage {
    fn drop(&mut self) {
        if self.active && self.free_raw() == 0 { self.active = false; }
    }
}

/// Read a guest file, keeping both borrowed buffers stable until completion.
pub fn read_file(name: &str, output: &mut [u8]) -> Result<usize, i64> {
    let name_ptr = name.as_ptr() as u64;
    loop {
        let mut result = name_ptr;
        unsafe { asm!("svc #0x140", inout("x0") result, in("x1") name.len() as u64,
            in("x2") output.as_mut_ptr() as u64, in("x3") output.len() as u64,
            options(nostack)); }
        match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), n => return Ok(n as usize) }
    }
}

/// Write a guest file; the kernel snapshots the input bytes on submission.
pub fn write_file(name: &str, input: &[u8]) -> Result<usize, i64> {
    let name_ptr = name.as_ptr() as u64;
    // An empty Rust slice may have a dangling pointer. The ABI still expects
    // a mapped address for zero-byte buffers, so reuse the validated name.
    let input_ptr = if input.is_empty() { name_ptr } else { input.as_ptr() as u64 };
    loop {
        let mut result = name_ptr;
        unsafe { asm!("svc #0x141", inout("x0") result, in("x1") name.len() as u64,
            in("x2") input_ptr, in("x3") input.len() as u64,
            options(nostack)); }
        match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), n => return Ok(n as usize) }
    }
}

/// Read at a byte offset; one call transfers at most 32 bytes. Zero means EOF.
pub fn read_at(name: &str, output: &mut [u8], offset: usize) -> Result<usize, i64> {
    let name_ptr = name.as_ptr() as u64;
    loop {
        let mut result = name_ptr;
        unsafe { asm!("svc #0x143", inout("x0") result, in("x1") name.len() as u64,
            in("x2") output.as_mut_ptr() as u64, in("x3") output.len() as u64,
            in("x4") offset as u64, options(nostack)); }
        match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), n => return Ok(n as usize) }
    }
}

/// Update a byte range without truncating the suffix. Gaps are rejected.
pub fn write_at(name: &str, input: &[u8], offset: usize) -> Result<usize, i64> {
    let name_ptr = name.as_ptr() as u64;
    let input_ptr = if input.is_empty() { name_ptr } else { input.as_ptr() as u64 };
    loop {
        let mut result = name_ptr;
        unsafe { asm!("svc #0x142", inout("x0") result, in("x1") name.len() as u64,
            in("x2") input_ptr, in("x3") input.len() as u64,
            in("x4") offset as u64, options(nostack)); }
        match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), n => return Ok(n as usize) }
    }
}

/// Return the last published tree generation, or None if no tree is open.
/// This is advisory: the tree is rechecked when a read is serviced.
pub fn tree_generation() -> Result<Option<u64>, i64> {
    let mut status: u64;
    let mut generation: u64;
    unsafe { asm!("svc #0x145", lateout("x0") status, lateout("x1") generation, options(nostack)); }
    if (status as i64) < 0 { return Err(status as i64); }
    if status != 0 { return Err(-5); }
    Ok(if generation == 0 { None } else { Some(generation) })
}

/// Read one chunk from the current actor's tree namespace at a generation.
/// A stale generation fails; the caller must not combine different versions.
pub fn tree_read_at(path: &str, output: &mut [u8], offset: usize, generation: u64) -> Result<usize, i64> {
    let path_ptr = path.as_ptr() as u64;
    loop {
        let mut result = path_ptr;
        unsafe { asm!("svc #0x144", inout("x0") result, in("x1") path.len() as u64,
            in("x2") output.as_mut_ptr() as u64, in("x3") output.len() as u64,
            in("x4") offset as u64, in("x5") generation, options(nostack)); }
        match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), n => return Ok(n as usize) }
    }
}
