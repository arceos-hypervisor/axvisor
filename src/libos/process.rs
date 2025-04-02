use alloc::sync::Arc;

use page_table_multiarch::PagingHandler;

use axaddrspace::AddrSpace;
use axaddrspace::npt::EPTEntry;

use crate::libos::def::ShadowPageTableMetadata;

pub const INIT_PROCESS_ID: usize = 0;

pub struct Process<H: PagingHandler> {
    pid: usize,
    /// For Stage-2 address translation, which translates guest physical address to host physical address,
    /// here we use a shadow page table, translating guest physical address to host physical address.
    ept_addrspace: AddrSpace<ShadowPageTableMetadata, EPTEntry, H>,
}

pub type ProcessRef<H> = Arc<Process<H>>;

impl<H: PagingHandler> Process<H> {
    pub fn new(
        pid: usize,
        ept_addrspace: AddrSpace<ShadowPageTableMetadata, EPTEntry, H>,
    ) -> ProcessRef<H> {
        info!("Create process: pid = {}", pid);
        Arc::new(Self { pid, ept_addrspace })
    }
}
