use std::os::arceos::modules::axhal;
use std::println;

use axaddrspace::GuestVirtAddr;
use axerrno::{AxResult, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;
use page_table_multiarch::PagingHandler;

use crate::libos::instance::InstanceRef;
use crate::libos::percpu::LibOSPerCpu;
use crate::vmm::VCpuRef;

pub struct InstanceCall<'a, H: PagingHandler> {
    vcpu: VCpuRef,
    pcpu: &'a LibOSPerCpu<H>,
    instance: InstanceRef,
    code: HyperCallCode,
    args: [u64; 6],
}

impl<'a, H: PagingHandler> InstanceCall<'a, H> {
    pub fn new(
        vcpu: VCpuRef,
        percpu: &'a LibOSPerCpu<H>,
        instance: InstanceRef,
        code: u64,
        args: [u64; 6],
    ) -> AxResult<Self> {
        let code = HyperCallCode::try_from(code as u32).map_err(|e| {
            warn!("Invalid hypercall code: {} e {:?}", code, e);
            ax_err_type!(InvalidInput)
        })?;

        Ok(Self {
            vcpu,
            pcpu: percpu,
            instance,
            code,
            args,
        })
    }

    pub fn execute(&self) -> HyperCallResult {
        // First, check if the vcpu is allowed to execute the hypercall.
        if self.code.is_privileged() ^ self.vcpu.get_arch_vcpu().guest_is_privileged() {
            debug!(
                "Vcpu[{}] execute hypercall {:?} from {}",
                self.vcpu.id(),
                self.code,
                if self.vcpu.get_arch_vcpu().guest_is_privileged() {
                    "Ring 0"
                } else {
                    "Ring 3"
                },
            );
        }

        match self.code {
            HyperCallCode::HyperVisorDebug => self.debug(),
            HyperCallCode::HDebug => self.debug(),
            HyperCallCode::HExitProcess => self.exit_process(self.args[0]),
            HyperCallCode::HClone => self.clone(),
            HyperCallCode::HRead => self.read(self.args[0], self.args[1], self.args[2]),
            HyperCallCode::HWrite => self.write(self.args[0], self.args[1], self.args[2]),
            HyperCallCode::HMMAP => {
                self.mmap(self.args[0], self.args[1], self.args[2], self.args[3])
            }
            _ => {
                unimplemented!();
            }
        }
    }
}

impl<'a, H: PagingHandler> InstanceCall<'a, H> {
    fn debug(&self) -> HyperCallResult {
        info!("HDebug {:#x?}", self.args);

        self.vcpu.get_arch_vcpu().dump();

        Ok(HyperCallCode::HDebug as usize)
    }

    /// Exit the instance with the given exit code.
    /// TODO: we may need to care about more context states.
    fn exit_process(&self, exit_code: u64) -> HyperCallResult {
        info!("HExitInstance code {exit_code:#x}");

        self.instance.remove_process(self.pcpu.current_ept_root())?;

        // DO NOT exit thread here, just mark the percpu as idle.
        // The thread will be exited in the next loop in `libos_vcpu_run`,
        // to let current `InstanceCall` to be dropped peacefully.
        if self.instance.processes.lock().len() == 0 {
            self.pcpu.mark_idle();
        }

        Ok(0)
    }

    fn clone(&self) -> HyperCallResult {
        info!("HClone");

        let new_pid = self.instance.handle_clone(self.pcpu.current_ept_root())?;
        if self.instance.eptp_list_dirty() {
            LibOSPerCpu::<H>::sync_eptp_list_region_on_all_vcpus(
                self.instance.id(),
                self.instance.get_eptp_list(),
            );
        }
        Ok(new_pid)
    }

    fn mmap(&self, addr: u64, len: u64, prot: u64, flags: u64) -> HyperCallResult {
        info!(
            "HMMAP addr:{:#x} len:{} prot:{} flags:{}",
            addr, len, prot, flags
        );
        Ok(0)
    }
}

impl<'a, H: PagingHandler> InstanceCall<'a, H> {
    fn read(&self, fd: u64, buffer_gva: u64, len: u64) -> HyperCallResult {
        info!(
            "HRead fd:{} buffer_gva:{:#x} len {:#x}",
            fd, buffer_gva, len
        );

        Ok(0)
    }

    fn write(&self, fd: u64, buffer_gva: u64, len: u64) -> HyperCallResult {
        info!(
            "HWrite fd:{} buffer_gva:{:#x} len {:#x}",
            fd, buffer_gva, len
        );
        let buffer = self.instance.read_from_guest(
            self.pcpu.current_ept_root(),
            GuestVirtAddr::from_usize(buffer_gva as usize),
            len as usize,
        )?;

        info!(
            "==== I[{}]P({})====\n",
            self.instance.id(),
            self.pcpu.current_process_id()
        );

        axhal::console::write_bytes(buffer.as_slice());

        println!("\n");
        info!(
            "xxxx I[{}]P({})xxxx",
            self.instance.id(),
            self.pcpu.current_process_id()
        );

        Ok(len as usize)
    }
}
