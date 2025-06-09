//! Inter-VM communication (IVC) module.
use alloc::vec::Vec;

use axaddrspace::HostPhysAddr;
use axerrno::AxResult;
use page_table_multiarch::PagingHandler;

pub struct IVCChannel<H: PagingHandler> {
    publisher_vm_id: usize,
    subscriber_vms: Vec<usize>,
    shared_region_base: HostPhysAddr,
    shared_region_size: usize,
    _phatom: core::marker::PhantomData<H>,
}

impl<H: PagingHandler> core::fmt::Debug for IVCChannel<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "IVCChannel(publisher[{}], subscribers {:?}, base: {:?}, size: {:#x})",
            self.publisher_vm_id,
            self.subscriber_vms,
            self.shared_region_base,
            self.shared_region_size
        )
    }
}

impl<H: PagingHandler> Drop for IVCChannel<H> {
    fn drop(&mut self) {
        // Free the shared region frame when the channel is dropped.
        debug!(
            "Dropping IVCChannel for VM[{}], shared region base: {:?}",
            self.publisher_vm_id, self.shared_region_base
        );
        H::dealloc_frame(self.shared_region_base);
    }
}

impl<H: PagingHandler> IVCChannel<H> {
    pub fn alloc(published_vm_id: usize, shared_region_size: usize) -> AxResult<Self> {
        // TODO: support larger shared region sizes with alloc_frames API.
        let shared_region_size = shared_region_size.min(4096);
        let shared_region_base = H::alloc_frame().ok_or_else(|| {
            axerrno::ax_err_type!(NoMemory, "Failed to allocate shared region frame")
        })?;

        Ok(Self {
            publisher_vm_id: published_vm_id,
            subscriber_vms: Vec::new(),
            shared_region_base,
            shared_region_size,
            _phatom: core::marker::PhantomData,
        })
    }

    pub fn base_hpa(&self) -> HostPhysAddr {
        self.shared_region_base
    }

    pub fn size(&self) -> usize {
        self.shared_region_size
    }

    pub fn publisher_vm_id(&self) -> usize {
        self.publisher_vm_id
    }

    pub fn add_subscriber(&mut self, subscriber_vm_id: usize) {
        if !self.subscriber_vms.contains(&subscriber_vm_id) {
            self.subscriber_vms.push(subscriber_vm_id);
        }
    }

    pub fn remove_subscriber(&mut self, subscriber_vm_id: usize) {
        if let Some(pos) = self
            .subscriber_vms
            .iter()
            .position(|&id| id == subscriber_vm_id)
        {
            self.subscriber_vms.remove(pos);
        }
    }

    pub fn subscribers(&self) -> &[usize] {
        &self.subscriber_vms
    }
}
