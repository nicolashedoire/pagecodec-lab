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

/// Create a directory in the current actor's tree namespace.
pub fn tree_mkdir(path: &str, generation: u64) -> Result<(), i64> {
    tree_directory_op(path, generation, true)
}

/// Remove a file or an empty directory in the current actor's namespace.
pub fn tree_remove(path: &str, generation: u64) -> Result<(), i64> {
    tree_directory_op(path, generation, false)
}

/// Read one fragment of an immediate child name. Zero means end of list;
/// otherwise byte 0 is kind (bit 0) and continuation (bit 7), followed by
/// name bytes. Repeat with the same child index and a larger name offset while
/// bit 7 is set, then advance the index. Generation is checked on each call.
pub fn tree_list_fragment(path: &str, index: usize, output: &mut [u8], generation: u64, name_offset: usize) -> Result<usize, i64> {
    let path_ptr = path.as_ptr() as u64;
    loop {
        let mut result = path_ptr;
        unsafe { asm!("svc #0x14c", inout("x0") result, in("x1") path.len() as u64,
            in("x2") index as u64, in("x3") output.as_mut_ptr() as u64,
            in("x4") output.len() as u64, in("x5") generation, in("x6") name_offset as u64,
            options(nostack)); }
        match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), n => return Ok(n as usize) }
    }
}

fn tree_directory_op(path: &str, generation: u64, mkdir: bool) -> Result<(), i64> {
    let path_ptr = path.as_ptr() as u64;
    loop {
        let mut result = path_ptr;
        unsafe {
            if mkdir { asm!("svc #0x14a", inout("x0") result, in("x1") path.len() as u64, in("x2") generation, options(nostack)); }
            else { asm!("svc #0x14b", inout("x0") result, in("x1") path.len() as u64, in("x2") generation, options(nostack)); }
        }
        match result as i64 {
            EAGAIN => core::hint::spin_loop(),
            n if n < 0 => return Err(n),
            0 => return Ok(()),
            _ => return Err(-5),
        }
    }
}

/// Sequential, incarnation-bound transfer. Abort explicitly or exit to reclaim it.
#[must_use]
pub struct TreeUpload { id: u64, next: usize, active: bool }

impl TreeUpload {
    pub fn begin(path: &str, size: usize, checksum: u64, generation: u64) -> Result<Self, i64> {
        let path_ptr = path.as_ptr() as u64;
        loop {
            let mut result = path_ptr;
            unsafe { asm!("svc #0x146", inout("x0") result, in("x1") path.len() as u64,
                in("x2") size as u64, in("x3") checksum, in("x4") generation,
                options(nostack)); }
            match result as i64 {
                EAGAIN => core::hint::spin_loop(),
                n if n < 0 => return Err(n),
                0 => return Err(-5),
                n => return Ok(Self { id: n as u64, next: 0, active: true }),
            }
        }
    }

    /// Append a nonempty chunk of at most 32 bytes.
    pub fn append(&mut self, bytes: &[u8]) -> Result<usize, i64> {
        if !self.active { return Err(-116); }
        loop {
            let mut result = self.next as u64;
            unsafe { asm!("svc #0x147", inout("x0") result, in("x1") bytes.as_ptr() as u64,
                in("x2") bytes.len() as u64, in("x3") self.id, options(nostack)); }
            match result as i64 {
                EAGAIN => core::hint::spin_loop(),
                n if n < 0 => return Err(n),
                n if n as usize == bytes.len() => {
                    self.next = self.next.checked_add(n as usize).ok_or(-75)?;
                    return Ok(n as usize);
                }
                _ => return Err(-5),
            }
        }
    }

    pub fn commit(&mut self) -> Result<(), i64> { self.finish(true) }
    pub fn abort(&mut self) -> Result<(), i64> { self.finish(false) }

    fn finish(&mut self, commit: bool) -> Result<(), i64> {
        if !self.active { return Err(-116); }
        loop {
            let mut result = self.id;
            unsafe {
                if commit { asm!("svc #0x148", inout("x0") result, options(nostack)); }
                else { asm!("svc #0x149", inout("x0") result, options(nostack)); }
            }
            match result as i64 {
                EAGAIN => core::hint::spin_loop(),
                n if n < 0 => return Err(n),
                0 => { self.active = false; return Ok(()); }
                _ => return Err(-5),
            }
        }
    }
}

/// One process-owned virtual surface. Destroy explicitly or exit to release it.
#[must_use]
pub struct Surface { id: u64, active: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceEvent { pub x: u32, pub y: u32, pub buttons: u32, pub source: u32 }

/// A reset event requires the application to clear all held-key state.
pub const KEY_RESET: u32 = u32::MAX;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent { pub code: u32, pub value: u32, pub source: u32 }

impl Surface {
    /// Public handle used for routing; it is not an authentication secret.
    pub fn id(&self) -> u64 { self.id }

    pub fn create(width: u32, height: u32) -> Result<Self, i64> {
        loop {
            let mut result = width as u64;
            unsafe { asm!("svc #0x1a0", inout("x0") result, in("x1") height as u64, options(nostack)); }
            match result as i64 {
                EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n),
                0 => return Err(-5), n => return Ok(Self { id: n as u64, active: true }),
            }
        }
    }

    /// Draw into the back buffer; `present` publishes it to the display.
    pub fn fill(&self, x: u32, y: u32, width: u32, height: u32, rgb: u32) -> Result<(), i64> {
        if !self.active { return Err(-116); }
        loop {
            let mut result = self.id;
            unsafe { asm!("svc #0x1a2", inout("x0") result, in("x1") x as u64,
                in("x2") y as u64, in("x3") width as u64, in("x4") height as u64,
                in("x5") rgb as u64, options(nostack)); }
            match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), 0 => return Ok(()), _ => return Err(-5) }
        }
    }

    pub fn present(&self) -> Result<(), i64> { self.simple(false) }

    pub fn destroy(&mut self) -> Result<(), i64> {
        self.simple(true)?; self.active = false; Ok(())
    }

    fn simple(&self, destroy: bool) -> Result<(), i64> {
        if !self.active { return Err(-116); }
        loop {
            let mut result = self.id;
            unsafe {
                if destroy { asm!("svc #0x1a1", inout("x0") result, options(nostack)); }
                else { asm!("svc #0x1a3", inout("x0") result, options(nostack)); }
            }
            match result as i64 { EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n), 0 => return Ok(()), _ => return Err(-5) }
        }
    }

    /// Return the next click, or None if the event queue is empty.
    pub fn poll_event(&self) -> Result<Option<SurfaceEvent>, i64> {
        if !self.active { return Err(-116); }
        let mut bytes = [0u8; 16];
        loop {
            let mut result = self.id;
            unsafe { asm!("svc #0x1a4", inout("x0") result, in("x1") bytes.as_mut_ptr() as u64, options(nostack)); }
            match result as i64 {
                EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n),
                0 => return Ok(None),
                16 => return Ok(Some(SurfaceEvent {
                    x: u32::from_le_bytes(bytes[0..4].try_into().map_err(|_| -5)?),
                    y: u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| -5)?),
                    buttons: u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| -5)?),
                    source: u32::from_le_bytes(bytes[12..16].try_into().map_err(|_| -5)?),
                })),
                _ => return Err(-5),
            }
        }
    }

    /// Poll an evdev-style key transition, or None when no key is queued.
    /// KEY_RESET is a state reset, not an ordinary key.
    pub fn poll_key(&self) -> Result<Option<KeyEvent>, i64> {
        if !self.active { return Err(-116); }
        let mut bytes = [0u8; 12];
        loop {
            let mut result = self.id;
            unsafe { asm!("svc #0x1a5", inout("x0") result, in("x1") bytes.as_mut_ptr() as u64, options(nostack)); }
            match result as i64 {
                EAGAIN => core::hint::spin_loop(), n if n < 0 => return Err(n),
                0 => return Ok(None),
                12 => return Ok(Some(KeyEvent {
                    code: u32::from_le_bytes(bytes[0..4].try_into().map_err(|_| -5)?),
                    value: u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| -5)?),
                    source: u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| -5)?),
                })),
                _ => return Err(-5),
            }
        }
    }
}
