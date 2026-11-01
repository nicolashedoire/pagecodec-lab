//! Bounded flat files for the integrated two-slot workspace snapshot.
//! Not a POSIX filesystem and not an authority or a durability boundary.

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use serde_json::{json, Value};

const MAX_FILES: usize = 8;
const MAX_NAME: usize = 32;
const MAX_FILE: usize = 512;
const MAX_TOTAL: usize = 800;

#[derive(Clone, Default)]
pub struct Files {
    entries: BTreeMap<String, Vec<u8>>,
    total: usize,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name != "." && name != ".."
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl Files {
    pub fn put(&mut self, name: &str, data: &[u8]) -> Result<(), &'static str> {
        if !valid_name(name) { return Err("invalid filename"); }
        if data.len() > MAX_FILE { return Err("file too large"); }
        let old_len = self.entries.get(name).map_or(0, Vec::len);
        if !self.entries.contains_key(name) && self.entries.len() == MAX_FILES { return Err("too many files"); }
        let next = self.total - old_len + data.len();
        if next > MAX_TOTAL { return Err("catalog too large"); }
        self.entries.insert(String::from(name), Vec::from(data));
        self.total = next;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }

    pub fn snapshot(&self) -> Value {
        let mut object = serde_json::Map::new();
        for (name, bytes) in &self.entries {
            let mut encoded = String::with_capacity(bytes.len() * 2);
            const HEX: &[u8; 16] = b"0123456789abcdef";
            for &b in bytes {
                encoded.push(HEX[usize::from(b >> 4)] as char);
                encoded.push(HEX[usize::from(b & 15)] as char);
            }
            object.insert(name.clone(), json!(encoded));
        }
        Value::Object(object)
    }

    pub fn from_snapshot(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() > MAX_FILES { return None; }
        let mut files = Self::default();
        for (name, value) in object {
            let hex = value.as_str()?.as_bytes();
            if hex.len() > MAX_FILE * 2 || hex.len() % 2 != 0 { return None; }
            let mut bytes = Vec::with_capacity(hex.len() / 2);
            for pair in hex.chunks_exact(2) {
                fn nibble(b: u8) -> Option<u8> {
                    match b { b'0'..=b'9' => Some(b - b'0'), b'a'..=b'f' => Some(b - b'a' + 10), _ => None }
                }
                bytes.push(nibble(pair[0])? << 4 | nibble(pair[1])?);
            }
            files.put(name, &bytes).ok()?;
        }
        Some(files)
    }
}
