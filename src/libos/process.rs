use page_table_multiarch::PagingHandler;

use axaddrspace::HostPhysAddr;
use axaddrspace::npt::{EPTEntry, EPTMetadata};
use axerrno::AxResult;

use super::gaddrspace::GuestAddrSpace;
use super::gpt::GuestEntry;

pub struct Process<H: PagingHandler> {
    /// The process ID in the instance.
    pid: usize,
    /// The guest address space of the process.
    guest_as: GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H>,
}

impl<H: PagingHandler> Process<H> {
    pub fn new(pid: usize, guest_as: GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H>) -> Self {
        info!("Create process: pid = {}", pid);
        Self { pid, guest_as }
    }

    pub fn pid(&self) -> usize {
        self.pid
    }

    pub fn set_pid(&mut self, pid: usize) {
        self.pid = pid;
    }

    pub fn addrspace_root(&self) -> HostPhysAddr {
        self.guest_as.ept_root_hpa()
    }

    pub fn addrspace(&self) -> &GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H> {
        &self.guest_as
    }

    pub fn addrspace_mut(&mut self) -> &mut GuestAddrSpace<EPTMetadata, EPTEntry, GuestEntry, H> {
        &mut self.guest_as
    }

    pub fn fork(&mut self, pid: usize) -> AxResult<Self> {
        let new_as = self.guest_as.fork()?;
        let new_process = Process::new(pid, new_as);
        Ok(new_process)
    }
}
