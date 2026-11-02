//! Temporary sequential EL0 tree-file uploads. These buffers do not commit
//! anything; the owner must revalidate authority/generation and persist a
//! separate tree candidate before reporting a successful commit.
use alloc::vec::Vec;

const ACTORS:usize=3;
const PATH_MAX:usize=crate::tree_store::MAX_PATH;
const CHUNK_MAX:usize=32;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Error {Invalid, Stale, Busy, TooLarge, OutOfMemory, Exhausted}

struct Stage {
    id:u64, incarnation:u64, path:[u8;PATH_MAX], path_len:usize,
    generation:u64, total:usize, checksum:u64, bytes:Vec<u8>,
}
impl Stage {
    fn clear(&mut self) {
        self.bytes.fill(0);self.bytes.clear();self.path.fill(0);
        self.path_len=0;self.generation=0;self.total=0;self.checksum=0;self.incarnation=0;self.id=0;
    }
}
impl Drop for Stage {fn drop(&mut self){self.clear()}}

pub struct Stages {items:[Option<Stage>;ACTORS], next_id:u64}
impl Stages {
    pub fn new()->Self {Self{items:core::array::from_fn(|_|None),next_id:1}}

    /// The kernel determines whether the exact process incarnation is live.
    /// A disappeared incarnation is revoked even if storage is blocked.
    pub fn reap(&mut self,mut live:impl FnMut(usize,u64)->bool) {
        for (actor,item) in self.items.iter_mut().enumerate() {
            if item.as_ref().is_some_and(|s|!live(actor,s.incarnation)) {*item=None;}
        }
    }
    fn get_mut(&mut self,actor:usize,incarnation:u64,id:u64)->Result<&mut Stage,Error> {
        let stage=self.items.get_mut(actor).ok_or(Error::Invalid)?.as_mut().ok_or(Error::Stale)?;
        if id==0 || stage.id!=id || incarnation==0 || stage.incarnation!=incarnation {return Err(Error::Stale)}
        Ok(stage)
    }
    fn get(&self,actor:usize,incarnation:u64,id:u64)->Result<&Stage,Error> {
        let stage=self.items.get(actor).ok_or(Error::Invalid)?.as_ref().ok_or(Error::Stale)?;
        if id==0 || stage.id!=id || incarnation==0 || stage.incarnation!=incarnation {return Err(Error::Stale)}
        Ok(stage)
    }
    pub fn begin(&mut self,actor:usize,incarnation:u64,path:&str,total:usize,checksum:u64,generation:u64)->Result<u64,Error> {
        let item=self.items.get_mut(actor).ok_or(Error::Invalid)?;
        if item.is_some() {return Err(Error::Busy)}
        let id=self.next_id;
        if id==0 || id>i64::MAX as u64 {return Err(Error::Exhausted)}
        if total>crate::tree_store::MAX_FILE {return Err(Error::TooLarge)}
        if incarnation==0 || generation==0 || path.is_empty() || path.len()>PATH_MAX {return Err(Error::Invalid)}
        let mut bytes=Vec::new();bytes.try_reserve_exact(total).map_err(|_|Error::OutOfMemory)?;
        let mut name=[0u8;PATH_MAX];name[..path.len()].copy_from_slice(path.as_bytes());
        *item=Some(Stage{id,incarnation,path:name,path_len:path.len(),generation,total,checksum,bytes});
        self.next_id=id+1;Ok(id)
    }
    pub fn append(&mut self,actor:usize,incarnation:u64,id:u64,offset:usize,chunk:&[u8])->Result<usize,Error> {
        let stage=self.get_mut(actor,incarnation,id)?;
        if chunk.is_empty() || chunk.len()>CHUNK_MAX || offset!=stage.bytes.len() {return Err(Error::Invalid)}
        if offset.checked_add(chunk.len()).is_none_or(|end|end>stage.total) {return Err(Error::Invalid)}
        // Capacity for the declared total was reserved before stage publication.
        stage.bytes.extend_from_slice(chunk);Ok(chunk.len())
    }
    pub fn content(&self,actor:usize,incarnation:u64,id:u64)->Result<(&str,&[u8],u64),Error> {
        let stage=self.get(actor,incarnation,id)?;
        if stage.bytes.len()!=stage.total {return Err(Error::Invalid)}
        let checksum=stage.bytes.iter().fold(0xcbf29ce484222325u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100000001b3));
        if checksum!=stage.checksum {return Err(Error::Invalid)}
        let path=core::str::from_utf8(&stage.path[..stage.path_len]).map_err(|_|Error::Invalid)?;
        Ok((path,&stage.bytes,stage.generation))
    }
    pub fn abort(&mut self,actor:usize,incarnation:u64,id:u64)->Result<(),Error> {
        self.get(actor,incarnation,id)?;self.items[actor]=None;Ok(())
    }
}
