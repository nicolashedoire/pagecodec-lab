//! Single, bounded, non-persistent ELF upload. FNV is an accidental-corruption
//! check, not authentication; the existing MCP endpoint has no credentials.
use alloc::vec::Vec;
use serde_json::{json,Value};

pub const MAX_IMAGE:usize=120+4096;
pub const MAX_CHUNK:usize=512;

pub struct Staging {
    bytes:Vec<u8>,
    next_id:u64,
    id:Option<u64>,
    expected_size:usize,
    checksum:u64,
    sealed:bool,
}
impl Staging {
    pub fn new()->Self {Self {bytes:Vec::with_capacity(MAX_IMAGE),next_id:1,id:None,expected_size:0,checksum:0,sealed:false}}
    pub fn begin(&mut self,size:usize,checksum:u64)->Result<u64,&'static str> {
        if self.id.is_some() {return Err("upload already active")}
        if size==0 || size>MAX_IMAGE {return Err("upload size outside bound")}
        let next=self.next_id.checked_add(1).ok_or("upload ids exhausted")?;
        let id=self.next_id;self.next_id=next;self.id=Some(id);self.expected_size=size;self.checksum=checksum;self.sealed=false;
        Ok(id)
    }
    fn check(&self,id:u64)->Result<(),&'static str> {
        if self.id!=Some(id) {Err("stale upload id")}else{Ok(())}
    }
    pub fn append(&mut self,id:u64,offset:usize,chunk:&[u8])->Result<usize,&'static str> {
        self.check(id)?;
        if self.sealed {return Err("upload already sealed")}
        if chunk.is_empty() || chunk.len()>MAX_CHUNK {return Err("upload chunk outside bound")}
        if offset!=self.bytes.len() {return Err("upload offset mismatch")}
        if offset.checked_add(chunk.len()).is_none_or(|n|n>self.expected_size) {return Err("upload exceeds declared size")}
        self.bytes.extend_from_slice(chunk);Ok(self.bytes.len())
    }
    pub fn seal(&mut self,id:u64)->Result<(),&'static str> {
        self.check(id)?;
        if self.sealed {return Err("upload already sealed")}
        if self.bytes.len()!=self.expected_size {return Err("upload incomplete")}
        let actual=self.bytes.iter().fold(0xcbf29ce484222325u64,|a,b|(a^u64::from(*b)).wrapping_mul(0x100000001b3));
        if actual!=self.checksum {return Err("upload checksum mismatch")}
        if crate::loader::parse(&self.bytes).is_none() {return Err("invalid staged ELF")}
        self.sealed=true;Ok(())
    }
    pub fn image(&self,id:u64)->Result<&[u8],&'static str> {
        self.check(id)?;if !self.sealed {return Err("upload not sealed")}Ok(&self.bytes)
    }
    pub fn reset(&mut self,id:u64)->Result<(),&'static str> {
        self.check(id)?;self.bytes.fill(0);self.bytes.clear();self.id=None;self.expected_size=0;self.checksum=0;self.sealed=false;Ok(())
    }
    pub fn status(&self)->Value {json!({"active":self.id.is_some(),"id":self.id.map(|v|alloc::format!("{}",v)),"size":self.expected_size,"received":self.bytes.len(),"sealed":self.sealed})}
}
