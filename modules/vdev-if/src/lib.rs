#![no_std]

extern crate alloc;

use core::{any::Any, ptr::NonNull, sync::atomic::Ordering};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::From, derive_more::Into,
)]
pub struct IrqNum(usize);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::From, derive_more::Into,
)]
pub struct GuestPhysAddr(usize);

pub trait VirtDeviceOp: Send + Any + 'static {
    fn name(&self) -> &str;
    fn invoke(&mut self);
}

pub trait VirtPlatformOp: Send + Clone + 'static {
    fn alloc_mmio_region(&self, addr: Option<GuestPhysAddr>, size: usize) -> Option<MmioRegion>;
    fn alloc_irq(&self, irq: Option<IrqNum>) -> Option<IrqNum>;
    fn send_irq(&self, irq: IrqNum);
}

pub struct MmioRegion {
    pub addr: GuestPhysAddr,
    pub access: NonNull<u8>,
    pub size: usize,
}

unsafe impl Send for MmioRegion {}

impl MmioRegion {
    pub fn as_slice_mut(&self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.access.as_ptr(), self.size) }
    }
}

pub struct IrqLineBus {
    
}
