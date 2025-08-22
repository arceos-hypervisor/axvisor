use axaddrspace::{
    GuestPhysAddrRange,
    device::{AccessWidth, DeviceAddrRange},
};
use axdevice_base::BaseDeviceOps;

pub struct MockTimer {
    // injector: RefCell<Option<Box<InterruptInjector>>>,
}

impl BaseDeviceOps<GuestPhysAddrRange> for MockTimer {
    fn emu_type(&self) -> axdevice_base::EmuDeviceType {
        axdevice_base::EmuDeviceType::Dummy // just a placeholder
    }

    fn address_range(&self) -> GuestPhysAddrRange {
        // a placeholder
        GuestPhysAddrRange::from_start_size(0x1234_0000.into(), 0x1000)
    }

    fn handle_read(
        &self,
        _addr: <GuestPhysAddrRange as DeviceAddrRange>::Addr,
        _width: AccessWidth,
    ) -> axerrno::AxResult<usize> {
        todo!()
    }

    fn handle_write(
        &self,
        _addr: <GuestPhysAddrRange as DeviceAddrRange>::Addr,
        _width: AccessWidth,
        _val: usize,
    ) -> axerrno::AxResult {
        todo!()
    }
}

impl MockTimer {
    // pub fn new() -> Self {
    //     Self {}
    // }

    // pub fn tick(&self) {
    //     use axvisor_api::vmm::*;
    //     inject_interrupt(current_vm_id(), current_vcpu_id(), 0x77);
    // }
}

unsafe impl Send for MockTimer {}
unsafe impl Sync for MockTimer {}
