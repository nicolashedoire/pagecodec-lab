//! Bounded, per-process allocator for the independent AArch64 guest ABI.
//!
//! This module expects `crate::guest_api::{HeapPage, PAGE_LIMIT, PAGE_SIZE}`.
//! It keeps the acquired `HeapPage` owners until process exit and never hands
//! the same page to an independent `HeapPage` owner. See README.md for the
//! threading, address-space and allocation limits.
use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};
use crate::guest_api::{HeapPage, PAGE_LIMIT, PAGE_SIZE};

// Metadata resides in the program's statically allocated data, not in a
// possibly unmapped user page. This bounds live allocations independently of
// payload capacity; exhaustion is reported with null just like page OOM.
const RECORD_LIMIT: usize = 256;
#[derive(Clone, Copy)]
struct Record { start: u16, len: u16, page: u8, live: bool }
const EMPTY: Record = Record { start: 0, len: 0, page: 0, live: false };

struct State {
    pages: [Option<HeapPage>; PAGE_LIMIT],
    bases: [usize; PAGE_LIMIT],
    records: [Record; RECORD_LIMIT],
}
impl State {
    const fn new() -> Self {
        Self {
            pages: [const { None }; PAGE_LIMIT],
            bases: [0; PAGE_LIMIT],
            records: [EMPTY; RECORD_LIMIT],
        }
    }

    // Pages are 4 KiB aligned, so an alignment <= PAGE_SIZE can be computed
    // using the offset. Every allocation stays wholly within one page.
    fn gap(&self, page: usize, size: usize, align: usize) -> Option<usize> {
        let mut candidate = 0usize;
        loop {
            let start = candidate.checked_add(align - 1)? & !(align - 1);
            let end = start.checked_add(size)?;
            if end > PAGE_SIZE { return None; }
            let mut next = start;
            for record in &self.records {
                if !record.live || record.page as usize != page { continue; }
                let left = record.start as usize;
                let right = left + record.len as usize;
                if start < right && left < end { next = next.max(right); }
            }
            if next == start { return Some(start); }
            candidate = next;
        }
    }

    fn publish(&mut self, record: usize, page: usize, start: usize, size: usize) -> *mut u8 {
        self.records[record] = Record { start: start as u16, len: size as u16, page: page as u8, live: true };
        (self.bases[page] + start) as *mut u8
    }

    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1);
        if size > PAGE_SIZE || layout.align() > PAGE_SIZE { return ptr::null_mut(); }
        let Some(record) = self.records.iter().position(|r| !r.live) else { return ptr::null_mut(); };
        for page in 0..PAGE_LIMIT {
            if self.pages[page].is_some() {
                if let Some(start) = self.gap(page, size, layout.align()) {
                    return self.publish(record, page, start, size);
                }
            }
        }
        for page in 0..PAGE_LIMIT {
            if self.pages[page].is_some() { continue; }
            // -17 can mean a separate HeapPage owns this index; -16 can mean
            // an in-flight zero-byte I/O pins the preceding boundary. Neither
            // authorizes us to access or reclaim that page. Try another index.
            let mut owner = match HeapPage::allocate_index(page) {
                Ok(owner) => owner,
                Err(-17 | -16) => continue,
                Err(_) => return ptr::null_mut(),
            };
            let base = owner.as_mut_slice().as_mut_ptr() as usize;
            self.bases[page] = base;
            self.pages[page] = Some(owner);
            // Empty page, size and alignment were validated above.
            let Some(start) = self.gap(page, size, layout.align()) else { return ptr::null_mut(); };
            return self.publish(record, page, start, size);
        }
        ptr::null_mut()
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, _layout: Layout) {
        let address = ptr as usize;
        for record in &mut self.records {
            if !record.live { continue; }
            let page = record.page as usize;
            if self.bases[page].checked_add(record.start as usize) == Some(address) {
                *record = EMPTY;
                return;
            }
        }
        // An invalid/double deallocation already violates GlobalAlloc's caller
        // contract; do not panic or recursively allocate while locked.
    }
}

pub struct GuestAllocator { locked: AtomicBool, state: UnsafeCell<State> }
impl GuestAllocator {
    pub const fn new() -> Self { Self { locked: AtomicBool::new(false), state: UnsafeCell::new(State::new()) } }
    fn lock(&self) -> Guard<'_> {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        Guard(self)
    }
}
// Safety: the atomic serializes access to state. Supported guests have one
// EL0 thread per address space and no reentrant EL0 signal handler. This is
// not a claim of SMP, signal or interrupt-handler allocator safety.
unsafe impl Sync for GuestAllocator {}
struct Guard<'a>(&'a GuestAllocator);
impl Drop for Guard<'_> { fn drop(&mut self) { self.0.locked.store(false, Ordering::Release); } }
unsafe impl GlobalAlloc for GuestAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = self.lock();
        unsafe { (&mut *self.state.get()).alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _guard = self.lock();
        unsafe { (&mut *self.state.get()).dealloc(ptr, layout) }
    }
}
