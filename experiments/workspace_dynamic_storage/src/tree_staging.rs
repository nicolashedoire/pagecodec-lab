//! One sequential, bounded tree file upload. This temporary buffer is not a
//! filesystem or transaction: only tree_disk::store commits its candidate.
use alloc::{string::String, vec::Vec};

pub const MAX_CHUNK:usize=512;

pub struct Staging {
    path:String, bytes:Vec<u8>, id:Option<u64>, next_id:u64,
    generation:u64, size:usize, checksum:u64, sealed:bool,
}
impl Staging {
    pub fn new()->Self {Self{path:String::new(),bytes:Vec::new(),id:None,next_id:1,generation:0,size:0,checksum:0,sealed:false}}
    pub fn begin(&mut self,path:&str,size:usize,checksum:u64,generation:u64)->Result<u64,&'static str> {
        if self.id.is_some() {return Err("tree upload already active")}
        if size>crate::tree_store::MAX_FILE {return Err("tree upload size outside bound")}
        if path.is_empty() || path.len()>crate::tree_store::MAX_PATH || generation==0 {return Err("invalid tree upload")}
        let next=self.next_id.checked_add(1).ok_or("tree upload ids exhausted")?;
        let mut candidate=String::new();candidate.try_reserve_exact(path.len()).map_err(|_|"tree upload allocation failed")?;candidate.push_str(path);
        self.bytes.try_reserve_exact(size).map_err(|_|"tree upload allocation failed")?;
        let id=self.next_id;self.path=candidate;self.next_id=next;self.id=Some(id);self.generation=generation;self.size=size;self.checksum=checksum;self.sealed=false;
        Ok(id)
    }
    fn check(&self,id:u64)->Result<(),&'static str> {if self.id!=Some(id){Err("stale tree upload id")}else{Ok(())}}
    pub fn append(&mut self,id:u64,offset:usize,chunk:&[u8])->Result<usize,&'static str> {
        self.check(id)?;if self.sealed{return Err("tree upload already sealed")}
        if chunk.is_empty() || chunk.len()>MAX_CHUNK {return Err("tree upload chunk outside bound")}
        if offset!=self.bytes.len() {return Err("tree upload offset mismatch")}
        if offset.checked_add(chunk.len()).is_none_or(|end|end>self.size) {return Err("tree upload exceeds declared size")}
        self.bytes.extend_from_slice(chunk);Ok(self.bytes.len())
    }
    pub fn seal(&mut self,id:u64)->Result<(),&'static str> {
        self.check(id)?;if self.sealed{return Err("tree upload already sealed")}
        if self.bytes.len()!=self.size{return Err("tree upload incomplete")}
        let hash=self.bytes.iter().fold(0xcbf29ce484222325u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100000001b3));
        if hash!=self.checksum{return Err("tree upload checksum mismatch")}
        self.sealed=true;Ok(())
    }
    pub fn content(&self,id:u64)->Result<(&str,&[u8],u64),&'static str> {
        self.check(id)?;if !self.sealed{return Err("tree upload not sealed")}
        Ok((&self.path,&self.bytes,self.generation))
    }
    pub fn reset(&mut self,id:u64)->Result<(),&'static str> {
        self.check(id)?;self.bytes.fill(0);self.bytes.clear();self.path.clear();self.id=None;self.generation=0;self.size=0;self.checksum=0;self.sealed=false;Ok(())
    }
}
