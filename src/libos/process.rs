use page_table_multiarch::PagingHandler;

use axaddrspace::HostPhysAddr;
use axerrno::AxResult;

use super::gaddrspace::EqAddrSpace;

pub struct Process<H: PagingHandler> {
    /// The process ID in the instance.
    pid: usize,
    /// The guest address space of the process.
    guest_as: EqAddrSpace<H>,
}

impl<H: PagingHandler> Process<H> {
    pub fn new(pid: usize, guest_as: EqAddrSpace<H>) -> Self {
        info!(
            "Instance [{}] create process: pid = {}",
            guest_as.instance_id(),
            pid
        );
        assert_eq!(pid, guest_as.process_id(), "Process ID mismatch");
        Self { pid, guest_as }
    }

    pub fn pid(&self) -> usize {
        self.pid
    }

    pub fn ept_root(&self) -> HostPhysAddr {
        self.guest_as.ept_root_hpa()
    }

    pub fn addrspace(&self) -> &EqAddrSpace<H> {
        &self.guest_as
    }

    pub fn addrspace_mut(&mut self) -> &mut EqAddrSpace<H> {
        &mut self.guest_as
    }

    pub fn fork(&mut self, pid: usize) -> AxResult<Self> {
        info!(
            "Instance [{}] Forking process: pid = {} from parent process {}",
            self.guest_as.instance_id(),
            pid,
            self.pid
        );

        let new_as = self.guest_as.fork(pid)?;
        let new_process = Process::new(pid, new_as);
        Ok(new_process)
    }
}
