//! Bounded flat files for the integrated two-slot workspace snapshot.
//! Not a POSIX filesystem and not an authority or a durability boundary.

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use serde_json::{json, Value};

const MAX_FILES: usize = 10;
const MAX_NAME: usize = 32;
const MAX_FILE: usize = 512;
const MAX_TOTAL: usize = 1100;

#[derive(Clone, Default)]
pub struct Files {
    entries: BTreeMap<String, Vec<u8>>,
    total: usize,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name != "." && name != ".."
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

// Match the persisted canonical lowercase format; reject malformed input
// before constructing a candidate file or changing the catalog.
pub fn decode_hex(hex: &str) -> Result<Vec<u8>, &'static str> {
    let hex=hex.as_bytes();
    if hex.len()>MAX_FILE*2 {return Err("file too large")}
    if hex.len()%2!=0 {return Err("hex length must be even")}
    fn nibble(b:u8)->Option<u8> {
        match b {b'0'..=b'9'=>Some(b-b'0'),b'a'..=b'f'=>Some(b-b'a'+10),_=>None}
    }
    let mut bytes=Vec::with_capacity(hex.len()/2);
    for pair in hex.chunks_exact(2) {
        let hi=nibble(pair[0]).ok_or("invalid lowercase hexadecimal")?;
        let lo=nibble(pair[1]).ok_or("invalid lowercase hexadecimal")?;
        bytes.push(hi<<4|lo);
    }
    Ok(bytes)
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

    pub fn get(&self, name: &str) -> Result<&[u8], &'static str> {
        if !valid_name(name) { return Err("invalid filename"); }
        self.entries.get(name).map(Vec::as_slice).ok_or("file not found")
    }

    // Loader receives a slice of the existing catalog, not an unbounded copy.
    // The caller must validate the program format and execute permissions.
    pub fn read_for_load(&self, name:&str, max:usize)->Result<&[u8], &'static str> {
        if max==0 || max>MAX_FILE {return Err("invalid loader bound")}
        let data=self.get(name)?;
        if data.is_empty() || data.len()>max {return Err("program size outside bound")}
        Ok(data)
    }

    pub fn remove(&mut self, name: &str) -> Result<(), &'static str> {
        if !valid_name(name) { return Err("invalid filename"); }
        let old = self.entries.remove(name).ok_or("file not found")?;
        self.total -= old.len();
        Ok(())
    }

    pub fn list(&self) -> Value {
        Value::Array(self.entries.iter().map(|(name, bytes)| json!({"name":name,"size":bytes.len()})).collect())
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
            let bytes=decode_hex(value.as_str()?).ok()?;
            files.put(name, &bytes).ok()?;
        }
        Some(files)
    }
}
