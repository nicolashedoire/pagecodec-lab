#![no_std]
#![forbid(unsafe_code)]

pub const CAPACITY:usize=16;
pub const RECORD_BYTES:usize=512;
const MAGIC:&[u8;8]=b"IOJRNL01";
const HEADER:usize=16;
const ENTRY:usize=8;

#[repr(u8)] #[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Category { DiskRead=1, DiskWrite=2, Display=3, Input=4 }
#[repr(u8)] #[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Actor { Human=1, Assistant=2, System=3 }
#[repr(u8)] #[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Outcome { Ok=1, Rejected=2, Failed=3 }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Record { pub sequence:u32, pub category:Category, pub actor:Actor, pub outcome:Outcome }
const EMPTY:Record=Record {sequence:0,category:Category::DiskRead,actor:Actor::System,outcome:Outcome::Ok};

#[derive(Clone,Copy,Debug,PartialEq,Eq)] pub struct Full;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum DecodeError { Length, Magic, Checksum, Count, Reserved, Sequence, Category, Actor, Outcome }

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Journal { len:usize, records:[Record;CAPACITY] }
impl Default for Journal {fn default()->Self {Self::new()}}
impl Journal {
    pub const fn new()->Self {Self {len:0,records:[EMPTY;CAPACITY]}}
    pub fn records(&self)->&[Record] {&self.records[..self.len]}
    /// Refuses overflow; never overwrites an older entry.
    pub fn append(&mut self, category:Category, actor:Actor, outcome:Outcome)->Result<(),Full> {
        if self.len==CAPACITY {return Err(Full)}
        self.records[self.len]=Record {sequence:self.len as u32+1,category,actor,outcome};
        self.len+=1;Ok(())
    }
    /// Fixed canonical sector. The checksum detects accidental corruption only.
    pub fn encode(&self)->[u8;RECORD_BYTES] {
        let mut output=[0u8;RECORD_BYTES];output[..8].copy_from_slice(MAGIC);output[8]=self.len as u8;
        for (i,r) in self.records().iter().enumerate() {
            let offset=HEADER+i*ENTRY;output[offset..offset+4].copy_from_slice(&r.sequence.to_le_bytes());
            output[offset+4]=r.category as u8;output[offset+5]=r.actor as u8;output[offset+6]=r.outcome as u8;
        }
        let sum=checksum(&output[..508]).to_le_bytes();output[508..].copy_from_slice(&sum);output
    }
    /// Decode only a complete canonical sector; replay is inspection, not I/O execution.
    pub fn decode(input:&[u8])->Result<Self,DecodeError> {
        if input.len()!=RECORD_BYTES {return Err(DecodeError::Length)}
        if &input[..8]!=MAGIC {return Err(DecodeError::Magic)}
        let expected=u32::from_le_bytes([input[508],input[509],input[510],input[511]]);
        if checksum(&input[..508])!=expected {return Err(DecodeError::Checksum)}
        let count=input[8] as usize;if count>CAPACITY {return Err(DecodeError::Count)}
        if input[9..HEADER].iter().any(|&v|v!=0) || input[HEADER+count*ENTRY..508].iter().any(|&v|v!=0) {return Err(DecodeError::Reserved)}
        let mut journal=Self::new();
        for i in 0..count {
            let offset=HEADER+i*ENTRY;
            let sequence=u32::from_le_bytes([input[offset],input[offset+1],input[offset+2],input[offset+3]]);
            if sequence!=i as u32+1 {return Err(DecodeError::Sequence)}
            if input[offset+7]!=0 {return Err(DecodeError::Reserved)}
            let category=match input[offset+4] {1=>Category::DiskRead,2=>Category::DiskWrite,3=>Category::Display,4=>Category::Input,_=>return Err(DecodeError::Category)};
            let actor=match input[offset+5] {1=>Actor::Human,2=>Actor::Assistant,3=>Actor::System,_=>return Err(DecodeError::Actor)};
            let outcome=match input[offset+6] {1=>Outcome::Ok,2=>Outcome::Rejected,3=>Outcome::Failed,_=>return Err(DecodeError::Outcome)};
            journal.append(category,actor,outcome).map_err(|_|DecodeError::Count)?;
        }
        Ok(journal)
    }
}
pub fn checksum(bytes:&[u8])->u32 {let mut h=2166136261u32;for &b in bytes {h=(h ^ u32::from(b)).wrapping_mul(16777619);}h}
