//! Two-NIC, QEMU-only bounded Ethernet transport. No IP/TCP or host network.
//! VirtIO DMA buffers are static and held until completion, even on failure.
use core::ptr::{addr_of,addr_of_mut,read_volatile,write_volatile};
use serde_json::{json,Value};
use virtio_drivers::{device::net::VirtIONetRaw,transport::mmio::MmioTransport};
use crate::VmHal;

const BUF:usize=1536;const MAX:usize=64;
const MAC:[[u8;6];2]=[[0x52,0x54,0,0x12,0x34,1],[0x52,0x54,0,0x12,0x34,2]];
const ETH:usize=14;const META:usize=4+16+8+8+1;
static mut TX_BYTES:[[u8;BUF];2]=[[0;BUF];2];
static mut RX_BYTES:[[u8;BUF];2]=[[0;BUF];2];
#[derive(Clone,Copy)]
pub struct Packet {pub source:u64,pub destination:u64,pub len:usize,pub bytes:[u8;MAX]}
static mut TX:[Option<Packet>;2]=[None;2];
static mut RX:[Option<Packet>;2]=[None;2];
static mut AVAILABLE:[bool;2]=[false;2];
static mut TX_SUBMITTED:[u64;2]=[0;2];
static mut TX_COMPLETED:[u64;2]=[0;2];
static mut RX_ACCEPTED:[u64;2]=[0;2];
static mut RX_DELIVERED:[u64;2]=[0;2];
static mut DROPPED:[u64;2]=[0;2];
fn inc(base:*mut [u64;2],actor:usize) {unsafe{let p=base.cast::<u64>().add(actor);write_volatile(p,read_volatile(p).saturating_add(1))}}
fn take_slot(base:*mut [Option<Packet>;2],actor:usize)->Option<Packet> {
 let p=unsafe{base.cast::<Option<Packet>>().add(actor)};
 let value=unsafe{read_volatile(p)};
 // Scrub the persistent queue payload while its variant is still Some;
 // copies held temporarily by the kernel stack are not covered.
 if let Some(mut old)=value {
  old.source=0;old.destination=0;old.len=0;old.bytes=[0;MAX];
  unsafe{write_volatile(p,Some(old))};
 }
 unsafe{write_volatile(p,None)};value
}
pub fn available()->bool {(unsafe{read_volatile(addr_of!(AVAILABLE))})==[true,true]}
pub fn enqueue(actor:usize,packet:Packet)->bool {
 if actor>=2 || !available() {return false}
 let p=unsafe{addr_of_mut!(TX).cast::<Option<Packet>>().add(actor)};
 if unsafe{read_volatile(p)}.is_some() {return false}
 unsafe{write_volatile(p,Some(packet))};true
}
pub fn peek(actor:usize)->Option<Packet> {if actor>=2 {None}else{unsafe{read_volatile(addr_of!(RX).cast::<Option<Packet>>().add(actor))}}}
pub fn consume(actor:usize)->Option<Packet> {if actor>=2 {return None};let r=take_slot(addr_of_mut!(RX),actor);if r.is_some(){inc(addr_of_mut!(RX_DELIVERED),actor)}r}
pub fn discard(actor:usize) {if actor<2 && take_slot(addr_of_mut!(RX),actor).is_some(){inc(addr_of_mut!(DROPPED),actor)}}
/// Remove software queues involving an exiting incarnation. DMA in-flight is
/// not cancelled; any resulting late frame fails boot/incarnation validation.
pub fn revoke(actor:usize,incarnation:u64) {
 if actor>=2{return}
 for index in 0..2 {
  for tx in [true,false] {
   let base=if tx {addr_of_mut!(TX)}else{addr_of_mut!(RX)};
   let p=unsafe{base.cast::<Option<Packet>>().add(index)};
   let matches=unsafe{read_volatile(p)}.is_some_and(|packet| {
    let source_actor=if tx {index}else{1-index};
    (source_actor==actor && packet.source==incarnation) || (1-source_actor==actor && packet.destination==incarnation)
   });
   if matches {take_slot(base,index);inc(addr_of_mut!(DROPPED),index)}
  }
 }
}
pub fn status()->Value {let tx=unsafe{&*addr_of!(TX)}.each_ref().map(Option::is_some);let rx=unsafe{&*addr_of!(RX)}.each_ref().map(Option::is_some);json!({"available":available(),"ports":unsafe{read_volatile(addr_of!(AVAILABLE))},"tx_queued":tx,"rx_queued":rx,"tx_submitted":unsafe{read_volatile(addr_of!(TX_SUBMITTED))},"tx_completed":unsafe{read_volatile(addr_of!(TX_COMPLETED))},"rx_accepted":unsafe{read_volatile(addr_of!(RX_ACCEPTED))},"rx_delivered":unsafe{read_volatile(addr_of!(RX_DELIVERED))},"dropped":unsafe{read_volatile(addr_of!(DROPPED))}})}

type Driver<'a>=VirtIONetRaw<VmHal,MmioTransport<'a>,8>;
struct Port<'a> {driver:Driver<'a>,rx:Option<u16>,tx:Option<(u16,usize)>,failed:bool}
pub struct Network<'a> {ports:[Option<Port<'a>>;2]}
impl<'a> Network<'a> {
 pub fn new()->Self {Self{ports:[None,None]}}
 /// Called once per enumerated VirtIO Network device; assign by MAC address.
 pub fn add(&mut self,transport:MmioTransport<'a>) {
  let Ok(mut driver)=Driver::new(transport) else{return};
  let Some(actor)=MAC.iter().position(|mac|*mac==driver.mac_address()) else{core::mem::forget(driver);return};
  if self.ports[actor].is_some(){core::mem::forget(driver);return}
  driver.disable_interrupts();
  let ptr=unsafe{addr_of_mut!(RX_BYTES).cast::<[u8;BUF]>().add(actor).cast::<u8>()};
  // SAFETY: a static per-device buffer stays valid and untouched until the
  // corresponding completion; Network stays alive for the entire VM boot.
  let buf=unsafe{core::slice::from_raw_parts_mut(ptr,BUF)};
  let Ok(token)=(unsafe{driver.receive_begin(buf)}) else{core::mem::forget(driver);return};
  self.ports[actor]=Some(Port{driver,rx:Some(token),tx:None,failed:false});
  unsafe{write_volatile(addr_of_mut!(AVAILABLE).cast::<bool>().add(actor),true)};
 }
 /// Poll each queue at most once; never waits for a device response.
 pub fn poll(&mut self) {
  for actor in 0..2 {
   let Some(port)=self.ports[actor].as_mut() else{continue};
   if port.failed {unsafe{write_volatile(addr_of_mut!(AVAILABLE).cast::<bool>().add(actor),false)};continue}
   if let Some((expected,len))=port.tx {
    if let Some(token)=port.driver.poll_transmit() {
     if token!=expected {port.failed=true}
     else {
      let ptr=unsafe{addr_of!(TX_BYTES).cast::<[u8;BUF]>().add(actor).cast::<u8>()};
      let bytes=unsafe{core::slice::from_raw_parts(ptr,len)};
      if unsafe{port.driver.transmit_complete(token,bytes)}.is_err(){port.failed=true}
      else {port.tx=None;inc(addr_of_mut!(TX_COMPLETED),actor)}
     }
    }
   }
   if !port.failed {
    if let Some(expected)=port.rx {
     if let Some(token)=port.driver.poll_receive() {
      if token!=expected {port.failed=true}
      else {
       let ptr=unsafe{addr_of_mut!(RX_BYTES).cast::<[u8;BUF]>().add(actor).cast::<u8>()};
       let bytes=unsafe{core::slice::from_raw_parts_mut(ptr,BUF)};
       match unsafe{port.driver.receive_complete(token,bytes)} {
        Ok((header,len))=>{
         port.rx=None;
         if let Some(end)=header.checked_add(len).filter(|end|*end<=BUF) {accept_frame(actor,&bytes[header..end])}else{inc(addr_of_mut!(DROPPED),actor)}
         // Previous request completed/unshared; this same static buffer may
         // now be posted again. No access until its next completion.
         match unsafe{port.driver.receive_begin(bytes)} {Ok(next)=>port.rx=Some(next),Err(_)=>port.failed=true}
        }
        Err(_)=>port.failed=true,
       }
      }
     }
    }
   }
   if !port.failed && port.tx.is_none() && port.driver.can_send() {
    let queued=unsafe{read_volatile(addr_of!(TX).cast::<Option<Packet>>().add(actor))};
    if let Some(packet)=queued {
     let live=crate::el0::network_identities();let boot=crate::boot_identity::identifier();
     if live.is_none_or(|ids|ids[actor]!=packet.source || ids[1-actor]!=packet.destination) || boot.is_none() {
      take_slot(addr_of_mut!(TX),actor);inc(addr_of_mut!(DROPPED),actor);
     } else {
      let ptr=unsafe{addr_of_mut!(TX_BYTES).cast::<[u8;BUF]>().add(actor).cast::<u8>()};
      let bytes=unsafe{core::slice::from_raw_parts_mut(ptr,BUF)};
      let Ok(header)=port.driver.fill_buffer_header(bytes) else{port.failed=true;unsafe{write_volatile(addr_of_mut!(AVAILABLE).cast::<bool>().add(actor),false)};continue};
      let total=(ETH+META+packet.len).max(60);let end=header+total;
      bytes[header..end].fill(0);let frame=&mut bytes[header..end];
      frame[..6].copy_from_slice(&MAC[1-actor]);frame[6..12].copy_from_slice(&MAC[actor]);frame[12..14].copy_from_slice(&[0x88,0xb5]);
      frame[14..18].copy_from_slice(b"CNET");frame[18..34].copy_from_slice(&boot.unwrap());
      frame[34..42].copy_from_slice(&packet.source.to_le_bytes());frame[42..50].copy_from_slice(&packet.destination.to_le_bytes());
      frame[50]=packet.len as u8;frame[51..51+packet.len].copy_from_slice(&packet.bytes[..packet.len]);
      match unsafe{port.driver.transmit_begin(&bytes[..end])} {
       Ok(token)=>{take_slot(addr_of_mut!(TX),actor);port.tx=Some((token,end));inc(addr_of_mut!(TX_SUBMITTED),actor)}
       Err(_)=>port.failed=true,
      }
     }
    }
   }
   if port.failed {unsafe{write_volatile(addr_of_mut!(AVAILABLE).cast::<bool>().add(actor),false)}}
  }
 }
}
fn accept_frame(actor:usize,frame:&[u8]) {
 let Some(boot)=crate::boot_identity::identifier() else{inc(addr_of_mut!(DROPPED),actor);return};
 let Some(ids)=crate::el0::network_identities() else{inc(addr_of_mut!(DROPPED),actor);return};
 if frame.len()<ETH+META || frame[..6]!=MAC[actor] || frame[6..12]!=MAC[1-actor] || frame[12..14]!=[0x88,0xb5] || &frame[14..18]!=b"CNET" || frame[18..34]!=boot {inc(addr_of_mut!(DROPPED),actor);return}
 let src=u64::from_le_bytes(frame[34..42].try_into().unwrap());let dst=u64::from_le_bytes(frame[42..50].try_into().unwrap());let len=usize::from(frame[50]);
 if src!=ids[1-actor] || dst!=ids[actor] || len==0 || len>MAX || 51+len>frame.len() {inc(addr_of_mut!(DROPPED),actor);return}
 let p=unsafe{addr_of_mut!(RX).cast::<Option<Packet>>().add(actor)};
 if unsafe{read_volatile(p)}.is_some() {inc(addr_of_mut!(DROPPED),actor);return}
 let mut packet=Packet{source:src,destination:dst,len,bytes:[0;MAX]};packet.bytes[..len].copy_from_slice(&frame[51..51+len]);
 unsafe{write_volatile(p,Some(packet))};inc(addr_of_mut!(RX_ACCEPTED),actor);
}
