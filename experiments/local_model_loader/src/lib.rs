#![no_std]
#![forbid(unsafe_code)]

// This compact SHA-256 implementation accepts at most 512 bytes and uses a
// fixed stack buffer. Its digest detects corruption, but does not authenticate.
const K:[u32;64]=[
0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2];
pub fn sha256(input:&[u8])->[u8;32] {
    assert!(input.len()<=512);
    let mut buf=[0u8;576];buf[..input.len()].copy_from_slice(input);buf[input.len()]=0x80;
    let n=(input.len()+9+63)/64*64;buf[n-8..n].copy_from_slice(&((input.len() as u64)*8).to_be_bytes());
    let mut h=[0x6a09e667u32,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
    for block in buf[..n].chunks_exact(64) {
        let mut w=[0u32;64];
        for i in 0..16 {w[i]=u32::from_be_bytes([block[4*i],block[4*i+1],block[4*i+2],block[4*i+3]]);}
        for i in 16..64 {let a=w[i-15].rotate_right(7)^w[i-15].rotate_right(18)^(w[i-15]>>3);let b=w[i-2].rotate_right(17)^w[i-2].rotate_right(19)^(w[i-2]>>10);w[i]=w[i-16].wrapping_add(a).wrapping_add(w[i-7]).wrapping_add(b);}
        let [mut a,mut b,mut c,mut d,mut e,mut f,mut g,mut z]=h;
        for i in 0..64 {let s1=e.rotate_right(6)^e.rotate_right(11)^e.rotate_right(25);let ch=(e&f)^(!e&g);let t1=z.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);let s0=a.rotate_right(2)^a.rotate_right(13)^a.rotate_right(22);let maj=(a&b)^(a&c)^(b&c);let t2=s0.wrapping_add(maj);z=g;g=f;f=e;e=d.wrapping_add(t1);d=c;c=b;b=a;a=t1.wrapping_add(t2);}
        for (v,x) in h.iter_mut().zip([a,b,c,d,e,f,g,z]) {*v=v.wrapping_add(x);}
    }
    let mut out=[0u8;32];for (i,x) in h.iter().enumerate(){out[i*4..i*4+4].copy_from_slice(&x.to_be_bytes());}out
}
#[derive(Debug,PartialEq,Eq)] pub enum Error {Magic,Version,Overflow,Shape,Length,Hash,Padding}
fn word(b:&[u8],offset:usize)->u32 {u32::from_le_bytes([b[offset],b[offset+1],b[offset+2],b[offset+3]])}
#[derive(Clone,Copy)] pub struct Model {w1:[[i8;3];4],b1:[i16;4],w2:[[i8;4];2],b2:[i16;2]}
pub fn parse(data:&[u8])->Result<Model,Error> {
    if data.len()!=512 {return Err(Error::Length)}
    if &data[..8]!=b"NSDENSE1" {return Err(Error::Magic)}
    if data[8..10]!=[1,0] || data[10..12]!=[0,0] {return Err(Error::Version)}
    let a=word(data,12) as usize;let b=word(data,16) as usize;let c=word(data,20) as usize;let n=word(data,24) as usize;
    let expected=a.checked_mul(b).and_then(|x|b.checked_mul(2).and_then(|y|x.checked_add(y))).and_then(|x|b.checked_mul(c).and_then(|y|x.checked_add(y))).and_then(|x|c.checked_mul(2).and_then(|y|x.checked_add(y))).ok_or(Error::Overflow)?;
    if (a,b,c)!=(3,4,2) {return Err(Error::Shape)}
    if n!=expected || n!=32 || 60usize.checked_add(n).is_none_or(|end|end>data.len()) {return Err(Error::Length)}
    if data[60+n..].iter().any(|&v|v!=0) {return Err(Error::Padding)}
    let mut hashed=[0u8;60];hashed[..28].copy_from_slice(&data[..28]);hashed[28..].copy_from_slice(&data[60..92]);
    if sha256(&hashed)!=data[28..60] {return Err(Error::Hash)}
    let mut m=Model {w1:[[0;3];4],b1:[0;4],w2:[[0;4];2],b2:[0;2]};let mut pos=60;
    for row in &mut m.w1 {for x in row {*x=data[pos] as i8;pos+=1;}}
    for x in &mut m.b1 {*x=i16::from_le_bytes([data[pos],data[pos+1]]);pos+=2;}
    for row in &mut m.w2 {for x in row {*x=data[pos] as i8;pos+=1;}}
    for x in &mut m.b2 {*x=i16::from_le_bytes([data[pos],data[pos+1]]);pos+=2;}
    Ok(m)
}
#[derive(Clone,Copy)] pub struct Input([i16;3]);
impl Input {pub fn new(v:[i16;3])->Option<Self> {if v.iter().any(|x| !(0..=100).contains(x)) {None} else {Some(Self(v))}}}
pub struct Output {pub hidden:[i32;4],pub scores:[i32;2]}
impl Model {
    pub fn infer(&self,input:Input)->Option<Output> {
        let mut h=[0i32;4];let mut s=[0i32;2];
        for (i,item) in h.iter_mut().enumerate(){let mut sum=self.b1[i] as i64;for j in 0..3 {sum=sum.checked_add((self.w1[i][j] as i64).checked_mul(input.0[j] as i64)?)?;}*item=i32::try_from(sum.max(0)).ok()?;}
        for (i,item) in s.iter_mut().enumerate(){let mut sum=self.b2[i] as i64;for (j,val) in h.iter().enumerate(){sum=sum.checked_add((self.w2[i][j] as i64).checked_mul(*val as i64)?)?;}*item=i32::try_from(sum).ok()?;}
        Some(Output {hidden:h,scores:s})
    }
}
