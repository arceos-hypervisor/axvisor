#![no_std]

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
}

pub trait VirtPlatformOp {
    fn alloc_mmio_region(
        &self,
        addr: Option<GuestPhysAddr>,
        size: usize,
        percpu: bool,
    ) -> Option<MmioRegion>;
    fn alloc_irq(&self, irq: Option<IrqNum>) -> Option<IrqNum>;
    fn invoke_irq(&self, irq: IrqNum);
}

pub struct MmioRegion {
    pub addr: GuestPhysAddr,
    pub access: NonNull<u8>,
    pub size: usize,
}

pub fn init(plat: &'static dyn VirtPlatformOp) {
    if INITED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    unsafe {
        GLOBAL_PLAT = plat;
    }
}

pub fn get_platform() -> &'static dyn VirtPlatformOp {
    if !INITED.load(Ordering::Acquire) {
        panic!("VirtPlatform Not initialized");
    }

    unsafe { GLOBAL_PLAT }
}

static mut GLOBAL_PLAT: &dyn VirtPlatformOp = &NopPlat;
static INITED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

struct NopPlat;

impl VirtPlatformOp for NopPlat {
    fn alloc_mmio_region(
        &self,
        _addr: Option<GuestPhysAddr>,
        _size: usize,
        _percpu: bool,
    ) -> Option<MmioRegion> {
        unimplemented!()
    }

    fn alloc_irq(&self, _irq: Option<IrqNum>) -> Option<IrqNum> {
        unimplemented!()
    }

    fn invoke_irq(&self, _irq: IrqNum) {
        unimplemented!()
    }
}
