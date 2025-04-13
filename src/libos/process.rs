use alloc::sync::Arc;

use page_table_multiarch::{PageSize, PagingHandler};

use axaddrspace::npt::EPTEntry;
use axaddrspace::{AddrSpace, GuestVirtAddr, HostPhysAddr, MappingFlags};

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

    pub fn set_pid(&mut self, pid: usize) {
        self.pid = pid;
    }

    pub fn pid(&self) -> usize {
        self.pid
    }

    pub fn addrspace(&self) -> &AddrSpace<ShadowPageTableMetadata, EPTEntry, H> {
        &self.ept_addrspace
    }

    pub fn translate_gva(
        &self,
        gva: GuestVirtAddr,
    ) -> Option<(HostPhysAddr, MappingFlags, PageSize)> {
        self.ept_addrspace.translate(gva)
    }
}
