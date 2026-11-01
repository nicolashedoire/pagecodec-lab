//! One nonblocking VirtIO entropy request for a public, per-boot control ID.
//!
//! QEMU supplies the bytes. This ID is not a secret or authentication token;
//! collision resistance depends on the host entropy source. A missing, silent,
//! short or failed device leaves process-control admission unavailable.
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use virtio_drivers::{device::common::Feature, queue::VirtQueue, transport::{mmio::MmioTransport, Transport}};
use crate::VmHal;

const SIZE:usize=16;
// This lives in guest kernel BSS for the entire boot, including if the device
// never completes. Never read or reuse it until the descriptor is popped.
static mut DMA_BYTES:[u8;SIZE]=[0;SIZE];
static mut ID:[u8;SIZE]=[0;SIZE];
static mut READY:bool=false;

/// The caller retains this object for the guest lifetime, even on failure or a
/// permanently silent device; the queue and DMA output must not be recycled.
pub struct BootEntropy<'a> {
    transport:MmioTransport<'a>,
    queue:VirtQueue<VmHal,8>,
    token:u16,
    pending:bool,
}
impl<'a> BootEntropy<'a> {
    pub fn begin(mut transport:MmioTransport<'a>)->Result<Self,()> {
        // No indirect descriptors or event indices are needed for one buffer.
        transport.begin_init(Feature::VERSION_1).map_err(|_|())?;
        let mut queue=VirtQueue::<VmHal,8>::new(&mut transport,0,false,false).map_err(|_|())?;
        transport.finish_init();
        let ptr=addr_of_mut!(DMA_BYTES).cast::<u8>();
        // SAFETY: the static buffer is permanently allocated, inaccessible to
        // anyone else while the descriptor is outstanding, and used by a
        // single one-shot request. DMA uses the kernel's identity map.
        let bytes=unsafe{core::slice::from_raw_parts_mut(ptr,SIZE)};
        let token=unsafe{queue.add(&[],&mut [bytes])}.map_err(|_|())?;
        transport.notify(0);
        Ok(Self{transport,queue,token,pending:true})
    }
    /// Poll once; never waits. After malformed completion the ID stays absent;
    /// retain this object and the DMA memory even if the device misbehaves.
    pub fn poll(&mut self) {
        if !self.pending || self.queue.peek_used()!=Some(self.token) {return}
        let ptr=addr_of_mut!(DMA_BYTES).cast::<u8>();
        // SAFETY: this is the same, still-live buffer and token given to add.
        // The used ring reported the token, and pop_used performs the DMA
        // unshare/recycle before we inspect the bytes.
        let bytes=unsafe{core::slice::from_raw_parts_mut(ptr,SIZE)};
        let result=unsafe{self.queue.pop_used(self.token,&[],&mut [bytes])};
        self.pending=false;
        if result!=Ok(SIZE as u32) {return}
        let mut id=[0u8;SIZE];
        for (i,byte) in id.iter_mut().enumerate() {*byte=unsafe{read_volatile(ptr.add(i))}}
        // Reject the conspicuous all-zero failure case; this is not an entropy
        // health test and does not validate a hostile or deterministic host.
        if id==[0;SIZE] {return}
        unsafe{write_volatile(addr_of_mut!(ID),id);write_volatile(addr_of_mut!(READY),true)};
        let _=&self.transport; // transport stays alive with the queue
    }
}
/// Public per-boot value, not a secret. Only safe to call on the sole CPU.
pub fn identifier()->Option<[u8;SIZE]> {
    if unsafe{read_volatile(addr_of!(READY))} {Some(unsafe{read_volatile(addr_of!(ID))})}else{None}
}
