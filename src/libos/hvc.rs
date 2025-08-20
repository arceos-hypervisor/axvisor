use std::os::arceos::modules::axhal;
use std::println;

use axaddrspace::GuestVirtAddr;
use axerrno::{AxResult, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;
use page_table_multiarch::PagingHandler;

use crate::libos::instance::{InstanceRef, shutdown_instance};
use crate::libos::percpu::EqOSPerCpu;
use crate::vmm::VCpuRef;

pub struct InstanceCall<'a, H: PagingHandler> {
    vcpu: VCpuRef,
    pcpu: &'a EqOSPerCpu<H>,
    instance: InstanceRef,
    code: HyperCallCode,
    args: [u64; 6],
}

impl<'a, H: PagingHandler> InstanceCall<'a, H> {
    pub fn new(
        vcpu: VCpuRef,
        percpu: &'a EqOSPerCpu<H>,
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
            HyperCallCode::HShutdownInstance => self.shutdown_instance(),
            HyperCallCode::HClone => self.clone(),
            HyperCallCode::HRead => self.read(self.args[0], self.args[1], self.args[2]),
            HyperCallCode::HWrite => self.write(self.args[0], self.args[1], self.args[2]),
            HyperCallCode::HAllocMMRegion => self.alloc_mm_region(self.args[0] as usize),
            HyperCallCode::HIVCGet => self.ivc_get(
                self.args[0] as u32,
                self.args[1] as usize,
                self.args[2] as usize,
                self.args[3] as usize,
            ),
            HyperCallCode::HClearGuestAreas => self.clear_guest_areas(),
            _ => {
                unimplemented!();
            }
        }
    }
}

impl<'a, H: PagingHandler> InstanceCall<'a, H> {
    fn debug(&self) -> HyperCallResult {
        info!(
            "CPU {} HDebug {:#x?}",
            self.pcpu.percpu_region().cpu_id,
            self.args
        );

        self.vcpu.get_arch_vcpu().dump();

        Ok(HyperCallCode::HDebug as usize)
    }

    /// Exit the instance with the given exit code.
    /// TODO: we may need to care about more context states.
    fn exit_process(&self, exit_code: u64) -> HyperCallResult {
        info!("HExitProcess code {exit_code:#x}");

        self.instance.remove_process(self.pcpu.current_ept_root())?;

        Ok(0)
    }

    fn shutdown_instance(&self) -> HyperCallResult {
        info!("HShutdownInstance");
        shutdown_instance(self.pcpu, &self.vcpu, &self.instance)?;
        Ok(0)
    }

    fn alloc_mm_region(&self, num_of_pages: usize) -> HyperCallResult {
        // info!("HAllocMMRegion num_of_pages {num_of_pages}");

        self.instance
            .alloc_mm_region(self.pcpu.current_ept_root(), num_of_pages)?;
        Ok(0)
    }

    fn clone(&self) -> HyperCallResult {
        info!("HClone");

        let new_pid = self.instance.handle_clone(self.pcpu.current_ept_root())?;
        if self.instance.eptp_list_dirty() {
            EqOSPerCpu::<H>::sync_eptp_list_region_on_all_vcpus(
                self.instance.id(),
                self.instance.get_eptp_list(),
            );
        }
        Ok(new_pid)
    }

    fn clear_guest_areas(&self) -> HyperCallResult {
        info!("HClearGuestAreas");
        self.instance
            .clear_guest_areas(self.pcpu.current_ept_root())?;
        Ok(0)
    }

    fn ivc_get(
        &self,
        key: u32,
        size: usize,
        flags: usize,
        shm_base_gva_ptr: usize,
    ) -> HyperCallResult {
        info!(
            "HIVCGet key: {:#x}, size: {:#x}, flags: {:#x}, shm_base_gva_ptr: {:#x}",
            key, size, flags, shm_base_gva_ptr
        );

        self.instance.process_ivc_get(
            self.pcpu.current_ept_root(),
            key,
            size,
            flags,
            shm_base_gva_ptr,
        )
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
