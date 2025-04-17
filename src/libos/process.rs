use alloc::sync::Arc;

use page_table_multiarch::PagingHandler;

use axaddrspace::npt::{EPTEntry, EPTMetadata};

use super::gaddrspace::GuestAddrSpace;
use super::gpt::GuestEntry;

pub const INIT_PROCESS_ID: usize = 0;

pub struct Process<H: PagingHandler> {
    /// The process ID in the instance.
    pid: usize,
    /// The guest address space of the process.
    guest_as: GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H>,
}

pub type ProcessRef<H> = Arc<Process<H>>;

impl<H: PagingHandler> Process<H> {
    pub fn new(
        pid: usize,
        guest_as: GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H>,
    ) -> ProcessRef<H> {
        info!("Create process: pid = {}", pid);
        Arc::new(Self { pid, guest_as })
    }

    pub fn addrspace(&self) -> &GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H> {
        &self.guest_as
    }
}
