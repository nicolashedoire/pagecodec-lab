//! Minimal bounded, process-owned CPU surfaces for the QEMU demonstration.
//! Slot/actor labels are not authenticated user identities. There is no window
//! manager, DMA sharing, typography, transparency, or remote-control session.
use alloc::vec::Vec;
use serde_json::{json,Value};
use alloc::format;

const ACTORS:usize=3;
pub const MAX_WIDTH:usize=256;
pub const MAX_HEIGHT:usize=112;
const QUEUE:usize=8;
const KEY_WORDS:usize=12; // evdev codes 0..0x2ff
const POSITIONS:[(usize,usize);ACTORS]=[(64,295),(456,295),(64,425)];

#[derive(Clone,Copy,Debug)]
pub enum Error {Invalid, NoSpace, NoMemory, Denied, Stale, Exhausted}
impl Error {pub fn errno(self)->i64 {match self {Self::Invalid=>-22,Self::NoSpace=>-28,Self::NoMemory=>-12,Self::Denied=>-13,Self::Stale=>-116,Self::Exhausted=>-75}}}

#[derive(Clone,Copy,PartialEq,Eq)]
pub enum Click {Miss,Queued,Full,Blocked}
#[derive(Clone,Copy,PartialEq,Eq)]
pub enum KeyDelivery {Queued,Reset,NoFocus,Unpaired,Invalid}

#[derive(Clone,Copy,Default)]
struct Event {x:u32,y:u32,buttons:u32,source:u32}
#[derive(Clone,Copy,Default)]
struct KeyEvent {code:u32,value:u32,source:u32}
struct Surface {
    id:u64,incarnation:u64,width:usize,height:usize,
    front:Vec<u32>,back:Vec<u32>,visible:bool,
    events:[Event;QUEUE],head:usize,len:usize,dropped:u64,
    keys:[KeyEvent;QUEUE],key_head:usize,key_len:usize,key_dropped:u64,
    delivered_down:[u64;KEY_WORDS],
}
impl Drop for Surface {
    fn drop(&mut self) {self.front.fill(0);self.back.fill(0);self.events.fill(Event::default());self.keys.fill(KeyEvent::default());self.delivered_down.fill(0);}
}

pub struct Surfaces {slots:[Option<Surface>;ACTORS],next_id:u64,human_focus:Option<(u64,u64)>,ai_focus:Option<(u64,u64)>}
impl Surfaces {
    pub fn new()->Self {Self{slots:core::array::from_fn(|_|None),next_id:1,human_focus:None,ai_focus:None}}
    /// Return true if a visible surface was removed and must be redrawn.
    pub fn reap(&mut self,mut live:impl FnMut(usize,u64)->bool)->bool {
        let mut redraw=false;
        for (actor,slot) in self.slots.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|s|!live(actor,s.incarnation)) {
                redraw|=slot.as_ref().is_some_and(|s|s.visible);*slot=None;
                if actor==0 {self.human_focus=None;}
                if actor==1 {self.ai_focus=None;}
            }
        }
        redraw
    }
    pub fn create(&mut self,actor:usize,incarnation:u64,width:u32,height:u32)->Result<u64,Error> {
        let slot=self.slots.get_mut(actor).ok_or(Error::Invalid)?;
        if slot.is_some(){return Err(Error::NoSpace)}
        if incarnation==0 || width==0 || height==0 || width as usize>MAX_WIDTH || height as usize>MAX_HEIGHT {return Err(Error::Invalid)}
        let count=(width as usize).checked_mul(height as usize).ok_or(Error::Invalid)?;
        let id=self.next_id;if id==0 || id>i64::MAX as u64 {return Err(Error::Exhausted)}
        let mut front=Vec::new();front.try_reserve_exact(count).map_err(|_|Error::NoMemory)?;front.resize(count,0);
        let mut back=Vec::new();back.try_reserve_exact(count).map_err(|_|Error::NoMemory)?;back.resize(count,0);
        *slot=Some(Surface{id,incarnation,width:width as usize,height:height as usize,front,back,visible:false,events:[Event::default();QUEUE],head:0,len:0,dropped:0,keys:[KeyEvent::default();QUEUE],key_head:0,key_len:0,key_dropped:0,delivered_down:[0;KEY_WORDS]});
        self.next_id=id+1;Ok(id)
    }
    fn locate(&self,actor:usize,incarnation:u64,id:u64)->Result<usize,Error> {
        if actor>=ACTORS || incarnation==0 || id==0 {return Err(Error::Invalid)}
        if let Some(s)=self.slots[actor].as_ref() {
            if s.id==id && s.incarnation==incarnation{return Ok(actor)}
        }
        if self.slots.iter().enumerate().any(|(i,s)|i!=actor && s.as_ref().is_some_and(|v|v.id==id)) {return Err(Error::Denied)}
        Err(Error::Stale)
    }
    pub fn fill(&mut self,actor:usize,incarnation:u64,id:u64,x:u32,y:u32,w:u32,h:u32,color:u32)->Result<(),Error> {
        let slot=self.locate(actor,incarnation,id)?;
        if w==0 || h==0{return Err(Error::Invalid)}
        let s=self.slots[slot].as_mut().ok_or(Error::Stale)?;
        // Coordinates and addition are clipped; only pixels in the back
        // buffer change. Alpha is ignored and rendering is fully opaque.
        let x0=(x as usize).min(s.width);let y0=(y as usize).min(s.height);
        let x1=(x as usize).saturating_add(w as usize).min(s.width);
        let y1=(y as usize).saturating_add(h as usize).min(s.height);
        for row in y0..y1 {s.back[row*s.width+x0..row*s.width+x1].fill(color&0x00ff_ffff);}
        Ok(())
    }
    /// Publish to the front buffer; the caller must redraw and flush the GPU
    /// before acknowledging PRESENT to the guest task.
    pub fn present(&mut self,actor:usize,incarnation:u64,id:u64)->Result<(),Error> {
        let slot=self.locate(actor,incarnation,id)?;let s=self.slots[slot].as_mut().ok_or(Error::Stale)?;
        s.front.copy_from_slice(&s.back);s.visible=true;Ok(())
    }
    /// Return whether a redraw is required after removing a visible surface.
    pub fn destroy(&mut self,actor:usize,incarnation:u64,id:u64)->Result<bool,Error> {
        let slot=self.locate(actor,incarnation,id)?;let visible=self.slots[slot].as_ref().is_some_and(|s|s.visible);
        self.slots[slot]=None;if slot==0 {self.human_focus=None}if slot==1 {self.ai_focus=None}Ok(visible)
    }
    /// Human source 0 targets slot 0, AI source 1 targets slot 1. These are
    /// demo routing conventions, not user identity or a remote access policy.
    /// Consumes any visible hit, even when the hit belongs to another
    /// actor or the bounded queue rejects the newest click. This prevents a
    /// click on an overlaid surface reaching a demo button underneath it.
    pub fn click(&mut self,source:u32,x:u32,y:u32)->Click {
        let permitted=match source {0=>0,1=>1,_=>return Click::Miss};
        for actor in (0..ACTORS).rev() {
            let (left,top)=POSITIONS[actor];let Some(s)=self.slots[actor].as_mut() else{continue};
            if !s.visible || (x as usize)<left || (y as usize)<top {continue}
            let local_x=x as usize-left;let local_y=y as usize-top;
            if local_x>=s.width || local_y>=s.height {continue}
            if actor!=permitted {if source==0 {self.human_focus=None}return Click::Blocked}
            if source==0 {self.human_focus=Some((s.id,s.incarnation));}
            if s.len==QUEUE {s.dropped=s.dropped.saturating_add(1);return Click::Full}
            if source==1 {self.ai_focus=Some((s.id,s.incarnation));}
            let index=(s.head+s.len)%QUEUE;s.events[index]=Event{x:local_x as u32,y:local_y as u32,buttons:1,source};s.len+=1;return Click::Queued
        }
        if source==0 {self.human_focus=None}
        Click::Miss
    }
    /// A successfully activated non-surface AI control releases AI focus.
    pub fn clear_ai_focus(&mut self) {self.ai_focus=None}
    pub fn poll(&mut self,actor:usize,incarnation:u64,id:u64)->Result<Option<[u8;16]>,Error> {
        let slot=self.locate(actor,incarnation,id)?;let s=self.slots[slot].as_mut().ok_or(Error::Stale)?;
        if s.len==0{return Ok(None)}
        let e=s.events[s.head];s.events[s.head]=Event::default();s.head=(s.head+1)%QUEUE;s.len-=1;
        let mut data=[0u8;16];
        for (i,value) in [e.x,e.y,e.buttons,e.source].into_iter().enumerate() {data[i*4..i*4+4].copy_from_slice(&value.to_le_bytes())}
        Ok(Some(data))
    }
    /// A release follows its original surface after focus loss if its press
    /// was delivered or remains queued. Saturation replaces pending events
    /// with a source-specific KEY_RESET; neither source is authenticated.
    fn push_key(s:&mut Surface,focused:bool,code:u32,value:u32,source:u32)->KeyDelivery {
        if value>2 || code as usize>=KEY_WORDS*64 {return KeyDelivery::Invalid}
        let word=code as usize/64;let mask=1u64<<(code%64);
        let mut down=s.delivered_down[word]&mask!=0;
        // Also retain ordering when the press has not yet been polled.
        for n in 0..s.key_len {
            let event=s.keys[(s.key_head+n)%QUEUE];
            if event.code==code {if event.value==1 {down=true} else if event.value==0 {down=false}}
        }
        if (value==0 || value==2) && !down {return KeyDelivery::Unpaired}
        if value==1 && down {return KeyDelivery::Unpaired}
        if !focused && !(value==0 && down) {return KeyDelivery::NoFocus}
        if s.key_len==QUEUE {
            // Notify the client to clear held keys after losing queued input.
            // This only helps if the client receives and honors the reset.
            s.key_dropped=s.key_dropped.saturating_add((s.key_len+1) as u64);
            s.keys.fill(KeyEvent::default());s.key_head=0;s.key_len=1;
            s.keys[0]=KeyEvent{code:u32::MAX,value:0,source};
            s.delivered_down.fill(0);return KeyDelivery::Reset
        }
        let i=(s.key_head+s.key_len)%QUEUE;s.keys[i]=KeyEvent{code,value,source};s.key_len+=1;KeyDelivery::Queued
    }
    pub fn human_key(&mut self,code:u32,value:u32)->bool {
        let Some(s)=self.slots[0].as_mut() else{self.human_focus=None;return false};
        if !s.visible {self.human_focus=None;return false}
        let focused=self.human_focus==Some((s.id,s.incarnation));
        Self::push_key(s,focused,code,value,0)==KeyDelivery::Queued
    }
    pub fn ai_key(&mut self,incarnation:u64,id:u64,code:u32,value:u32)->Result<KeyDelivery,Error> {
        let slot=self.locate(1,incarnation,id)?;
        let s=self.slots[slot].as_mut().ok_or(Error::Stale)?;
        if !s.visible {return Ok(KeyDelivery::NoFocus)}
        let focused=self.ai_focus==Some((s.id,s.incarnation));
        Ok(Self::push_key(s,focused,code,value,1))
    }
    pub fn poll_key(&mut self,actor:usize,incarnation:u64,id:u64)->Result<Option<[u8;12]>,Error> {
        let slot=self.locate(actor,incarnation,id)?;let s=self.slots[slot].as_mut().ok_or(Error::Stale)?;
        if s.key_len==0 {return Ok(None)}
        let e=s.keys[s.key_head];s.keys[s.key_head]=KeyEvent::default();s.key_head=(s.key_head+1)%QUEUE;s.key_len-=1;
        if e.code==u32::MAX {s.delivered_down.fill(0)} else {
            let word=e.code as usize/64;let mask=1u64<<(e.code%64);
            if e.value==1 {s.delivered_down[word]|=mask} else if e.value==0 {s.delivered_down[word]&=!mask}
        }
        let mut data=[0u8;12];
        for (i,value) in [e.code,e.value,e.source].into_iter().enumerate() {data[i*4..i*4+4].copy_from_slice(&value.to_le_bytes())}
        Ok(Some(data))
    }
    pub fn compose(&self,buf:&mut [u8],width:usize,height:usize) {
        for (actor,slot) in self.slots.iter().enumerate() {
            let Some(s)=slot.as_ref() else{continue};if !s.visible {continue}
            let (left,top)=POSITIONS[actor];
            for y in 0..s.height {for x in 0..s.width {
                let Some(px)=left.checked_add(x) else{continue};let Some(py)=top.checked_add(y) else{continue};
                if px>=width || py>=height {continue}
                let Some(i)=py.checked_mul(width).and_then(|n|n.checked_add(px)).and_then(|n|n.checked_mul(4)) else{continue};
                let Some(end)=i.checked_add(4) else{continue};
                if let Some(dst)=buf.get_mut(i..end) {let color=s.front[y*s.width+x];dst.copy_from_slice(&[color as u8,(color>>8) as u8,(color>>16) as u8,255]);}
            }}
        }
    }
    pub fn status(&self)->Value {
        let slots:Vec<Value>=self.slots.iter().enumerate().filter_map(|(actor,s)|s.as_ref().map(|s| {
            let (x,y)=POSITIONS[actor];json!({"slot":actor,"incarnation":format!("{}",s.incarnation),"id":format!("{}",s.id),"x":x,"y":y,"width":s.width,"height":s.height,"visible":s.visible,"queued":s.len,"dropped":s.dropped,"keyboard_focused":actor==0 && self.human_focus==Some((s.id,s.incarnation)),"ai_keyboard_focused":actor==1 && self.ai_focus==Some((s.id,s.incarnation)),"keyboard_queued":s.key_len,"keyboard_dropped":s.key_dropped})
        })).collect();json!({"surfaces":slots})
    }
}
