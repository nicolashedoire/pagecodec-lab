//! Bounded in-memory tree and strict binary interchange format.
//!
//! This module does not write to a device, commit snapshots, authenticate data,
//! or migrate an existing disk by itself. Build a fresh tree as a migration
//! candidate, explicitly map legacy files and document bytes into it, encode,
//! verify, and commit it separately before switching readers. Callers must not
//! infer crash safety or integrity from successful encoding/decoding.
//!
//! V1 is little-endian: `TRS1`, u16 version=1, u16 flags=0, u32 entry count;
//! then sorted records: u8 kind (0 directory, 1 file), u8 reserved=0, u16
//! path length, u32 content length, followed by path and content bytes. Root
//! `/` is an explicit directory and the first record. There is no checksum.

use alloc::vec::Vec;
use core::cmp::Ordering;

pub const MAX_ENTRIES: usize = 64; // includes root
pub const MAX_PATH: usize = 255;
pub const MAX_FILE: usize = 16 * 1024;
pub const MAX_CONTENT: usize = 64 * 1024;
const MAGIC: &[u8; 4] = b"TRS1";
const HEADER: usize = 12;
const RECORD: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidPath, NotFound, NotDirectory, IsDirectory, AlreadyExists,
    NotEmpty, Root, TooManyEntries, FileTooLarge, StorageFull, Hole,
    Overflow, OutOfMemory, InvalidFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Directory, File }

#[derive(Debug, PartialEq, Eq)]
enum Value { Directory, File(Vec<u8>) }

#[derive(Debug, PartialEq, Eq)]
struct Entry { path: Vec<u8>, value: Value }

#[derive(Debug, PartialEq, Eq)]
pub struct TreeStore { entries: Vec<Entry>, content_bytes: usize }

fn bytes(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(data.len()).map_err(|_| Error::OutOfMemory)?;
    out.extend_from_slice(data);
    Ok(out)
}

// Canonical absolute ASCII paths; no empty, dot, dot-dot, or trailing segment.
fn validate(path: &str) -> Result<(), Error> {
    let p = path.as_bytes();
    if p.is_empty() || p.len() > MAX_PATH || p[0] != b'/' { return Err(Error::InvalidPath); }
    if p == b"/" { return Ok(()); }
    for segment in p[1..].split(|&b| b == b'/') {
        if segment.is_empty() || segment == b"." || segment == b".." ||
            !segment.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            return Err(Error::InvalidPath);
        }
    }
    Ok(())
}

fn parent(path: &str) -> &str {
    let i = path.rfind('/').unwrap_or(0);
    if i == 0 { "/" } else { &path[..i] }
}

impl TreeStore {
    pub fn new() -> Result<Self, Error> {
        let mut entries = Vec::new();
        entries.try_reserve_exact(1).map_err(|_| Error::OutOfMemory)?;
        entries.push(Entry { path: bytes(b"/")?, value: Value::Directory });
        Ok(Self { entries, content_bytes: 0 })
    }

    fn find(&self, path: &str) -> Result<usize, usize> {
        self.entries.binary_search_by(|e| e.path.as_slice().cmp(path.as_bytes()))
    }

    fn check_parent(&self, path: &str) -> Result<(), Error> {
        let index = self.find(parent(path)).map_err(|_| Error::NotFound)?;
        if !matches!(self.entries[index].value, Value::Directory) { return Err(Error::NotDirectory); }
        Ok(())
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), Error> {
        validate(path)?;
        if path == "/" { return Err(Error::AlreadyExists); }
        let index = match self.find(path) { Ok(_) => return Err(Error::AlreadyExists), Err(i) => i };
        self.check_parent(path)?;
        if self.entries.len() == MAX_ENTRIES { return Err(Error::TooManyEntries); }
        let name = bytes(path.as_bytes())?;
        self.entries.try_reserve(1).map_err(|_| Error::OutOfMemory)?;
        self.entries.insert(index, Entry { path: name, value: Value::Directory });
        Ok(())
    }

    /// Create or replace one complete file. Directories cannot be replaced.
    pub fn put(&mut self, path: &str, data: &[u8]) -> Result<(), Error> {
        validate(path)?;
        if data.len() > MAX_FILE { return Err(Error::FileTooLarge); }
        let position = self.find(path);
        let old_len = match position {
            Ok(i) => match &self.entries[i].value { Value::Directory => return Err(Error::IsDirectory), Value::File(x) => x.len() },
            Err(_) => { self.check_parent(path)?; if self.entries.len() == MAX_ENTRIES { return Err(Error::TooManyEntries); } 0 },
        };
        let total = self.content_bytes.checked_sub(old_len).and_then(|n| n.checked_add(data.len())).ok_or(Error::Overflow)?;
        if total > MAX_CONTENT { return Err(Error::StorageFull); }
        let value = bytes(data)?;
        match position {
            Ok(i) => self.entries[i].value = Value::File(value),
            Err(i) => {
                let name = bytes(path.as_bytes())?;
                self.entries.try_reserve(1).map_err(|_| Error::OutOfMemory)?;
                self.entries.insert(i, Entry { path: name, value: Value::File(value) });
            }
        }
        self.content_bytes = total;
        Ok(())
    }

    pub fn get(&self, path: &str) -> Result<&[u8], Error> {
        validate(path)?;
        let i = self.find(path).map_err(|_| Error::NotFound)?;
        match &self.entries[i].value { Value::Directory => Err(Error::IsDirectory), Value::File(data) => Ok(data) }
    }

    pub fn read_at(&self, path: &str, offset: usize, capacity: usize) -> Result<&[u8], Error> {
        let data = self.get(path)?;
        let start = offset.min(data.len());
        let count = capacity.min(data.len() - start);
        Ok(&data[start..start + count])
    }

    /// Replace a range and retain the suffix; gaps are not permitted.
    /// An empty write at offset zero creates a missing empty file.
    pub fn write_at(&mut self, path: &str, offset: usize, value: &[u8]) -> Result<(), Error> {
        validate(path)?;
        let end = offset.checked_add(value.len()).ok_or(Error::Overflow)?;
        if end > MAX_FILE { return Err(Error::FileTooLarge); }
        let old = match self.get(path) { Ok(v) => v, Err(Error::NotFound) => &[], Err(e) => return Err(e) };
        if offset > old.len() { return Err(Error::Hole); }
        let size = old.len().max(end);
        let mut next = Vec::new();
        next.try_reserve_exact(size).map_err(|_| Error::OutOfMemory)?;
        next.extend_from_slice(old);
        next.resize(size, 0);
        next[offset..end].copy_from_slice(value);
        self.put(path, &next)
    }

    /// Remove only a file or empty directory; never recursively deletes data.
    pub fn remove(&mut self, path: &str) -> Result<(), Error> {
        validate(path)?;
        if path == "/" { return Err(Error::Root); }
        let i = self.find(path).map_err(|_| Error::NotFound)?;
        if matches!(self.entries[i].value, Value::Directory) {
            let prefix = path.as_bytes();
            if self.entries.iter().any(|e| e.path.len() > prefix.len() && e.path.starts_with(prefix) && e.path[prefix.len()] == b'/') {
                return Err(Error::NotEmpty);
            }
        }
        let removed = self.entries.remove(i);
        if let Value::File(data) = removed.value { self.content_bytes -= data.len(); }
        Ok(())
    }

    pub fn entry_count(&self) -> usize { self.entries.len() }
    pub fn content_bytes(&self) -> usize { self.content_bytes }

    /// Enumerate canonical paths in byte order without allocating. File bytes
    /// are obtained separately with `get`; directories have size zero.
    pub fn entries(&self) -> impl Iterator<Item = (&str, Kind, usize)> {
        self.entries.iter().map(|entry| {
            let path = core::str::from_utf8(&entry.path).expect("validated ASCII path");
            match &entry.value {
                Value::Directory => (path, Kind::Directory, 0),
                Value::File(data) => (path, Kind::File, data.len()),
            }
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut len = HEADER;
        for e in &self.entries {
            let n = match &e.value { Value::Directory => 0, Value::File(x) => x.len() };
            len = len.checked_add(RECORD).and_then(|x| x.checked_add(e.path.len())).and_then(|x| x.checked_add(n)).ok_or(Error::Overflow)?;
        }
        let mut out = Vec::new();
        out.try_reserve_exact(len).map_err(|_| Error::OutOfMemory)?;
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            let (kind, value): (u8, &[u8]) = match &e.value { Value::Directory => (0, &[]), Value::File(x) => (1, x) };
            out.push(kind); out.push(0);
            out.extend_from_slice(&(e.path.len() as u16).to_le_bytes());
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(&e.path); out.extend_from_slice(value);
        }
        Ok(out)
    }

    /// Decode only a complete, canonical encoding; rejects trailing data,
    /// duplicates, bad ordering, unknown kinds/flags, and missing parents.
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() < HEADER || input.get(..4) != Some(MAGIC) ||
            input[4..8] != [1, 0, 0, 0] { return Err(Error::InvalidFormat); }
        let count = u32::from_le_bytes(input[8..12].try_into().map_err(|_| Error::InvalidFormat)?) as usize;
        if !(1..=MAX_ENTRIES).contains(&count) { return Err(Error::InvalidFormat); }
        let mut tree = Self::new()?;
        let mut pos = HEADER;
        for index in 0..count {
            let record_end = pos.checked_add(RECORD).ok_or(Error::InvalidFormat)?;
            let h = input.get(pos..record_end).ok_or(Error::InvalidFormat)?;
            let kind = h[0];
            if kind > 1 || h[1] != 0 { return Err(Error::InvalidFormat); }
            let path_len = u16::from_le_bytes([h[2],h[3]]) as usize;
            let data_len = u32::from_le_bytes([h[4],h[5],h[6],h[7]]) as usize;
            if path_len == 0 || path_len > MAX_PATH || data_len > MAX_FILE || (kind == 0 && data_len != 0) { return Err(Error::InvalidFormat); }
            let path_end = record_end.checked_add(path_len).ok_or(Error::InvalidFormat)?;
            let end = path_end.checked_add(data_len).ok_or(Error::InvalidFormat)?;
            let name = core::str::from_utf8(input.get(record_end..path_end).ok_or(Error::InvalidFormat)?).map_err(|_| Error::InvalidFormat)?;
            let value = input.get(path_end..end).ok_or(Error::InvalidFormat)?;
            validate(name).map_err(|_| Error::InvalidFormat)?;
            if index == 0 {
                if name != "/" || kind != 0 { return Err(Error::InvalidFormat); }
            } else {
                if tree.entries.last().is_none_or(|e| e.path.as_slice().cmp(name.as_bytes()) != Ordering::Less) { return Err(Error::InvalidFormat); }
                let result = if kind == 0 { tree.mkdir(name) } else { tree.put(name, value) };
                result.map_err(|e| if e == Error::OutOfMemory { e } else { Error::InvalidFormat })?;
            }
            pos = end;
        }
        if pos != input.len() { return Err(Error::InvalidFormat); }
        Ok(tree)
    }
}
