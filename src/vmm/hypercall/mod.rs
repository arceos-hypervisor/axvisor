use core::sync::atomic::{AtomicUsize, Ordering};

use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;

use equation_defs::{GuestMappingType, InstanceType};

use crate::libos::def::get_contents_from_shared_pages;
use crate::vmm::{VCpuRef, VMRef};

pub struct HyperCall {
    vcpu: VCpuRef,
    vm: VMRef,
    code: HyperCallCode,
    args: [u64; 6],
}

impl HyperCall {
    pub fn new(vcpu: VCpuRef, vm: VMRef, code: u64, args: [u64; 6]) -> AxResult<Self> {
        let code = HyperCallCode::try_from(code as u32).map_err(|e| {
            warn!("Invalid hypercall code: {} e {:?}", code, e);
            ax_err_type!(InvalidInput)
        })?;

        Ok(Self {
            vcpu,
            vm,
            code,
            args,
        })
    }

    pub fn execute(&self) -> HyperCallResult {
        // First, check if the vcpu is allowed to execute the hypercall.
        if self.code.is_privileged() ^ self.vcpu.get_arch_vcpu().guest_is_privileged() {
            warn!(
                "{} vcpu trying to execute {} hypercall {:?}",
                if self.vcpu.get_arch_vcpu().guest_is_privileged() {
                    "Privileged"
                } else {
                    "Unprivileged"
                },
                if self.code.is_privileged() {
                    "privileged"
                } else {
                    "unprivileged"
                },
                self.code
            );
            return ax_err!(PermissionDenied);
        }
        debug!("VMM Hypercall: {:?} args: {:x?}", self.code, self.args);

        if self.vcpu.get_arch_vcpu().guest_is_privileged() {
            self.execute_privileged()
        } else {
            self.execute_unprivileged()
        }
    }

    fn execute_privileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HypervisorDisable => self.hypervisor_disable(),
            HyperCallCode::HyperVisorDebug => self.debug(),
            _ => {
                unimplemented!()
            }
        }
    }

    fn execute_unprivileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HDebug => self.debug(),
            HyperCallCode::HInitShim => self.init_shim(),
            HyperCallCode::HCreateInstance => self.create_instance(
                self.args[0].into(),
                self.args[1].into(),
                self.args[2],
                self.args[3],
                self.args[4],
            ),
            _ => {
                unimplemented!();
            }
        }
    }
}

impl HyperCall {
    fn hypervisor_disable(&self) -> HyperCallResult {
        info!("HypervisorDisable");

        let reserved_cpus = crate::vmm::config::get_reserved_cpus();

        static TRY_DISABLED_CPUS: AtomicUsize = AtomicUsize::new(0);

        if TRY_DISABLED_CPUS.fetch_add(1, Ordering::SeqCst) == 0 {
            // We need to disable virtualization on CPUs belonging to ArceOS,
            // then shutdown these CPUs.
            crate::hal::disable_virtualization_on_remaining_cores()?;

            // Add `1` to TRY_DISABLED_CPUS to indicate that virtualization on other CPUs
            // has been disabled.
            TRY_DISABLED_CPUS.fetch_add(1, Ordering::SeqCst);
        }

        // Wait for all CPUs to trgger the hypervisor disable HVC from Linux.
        // Wait for all other CPUs to disable virtualization.
        while TRY_DISABLED_CPUS.load(Ordering::SeqCst) < reserved_cpus + 1 {
            core::hint::spin_loop();
        }

        crate::hal::disable_virtualization(self.vcpu.clone(), 0)?;

        unreachable!("HypervisorDisable should not reach here");
    }

    fn debug(&self) -> HyperCallResult {
        info!("HDebug {:#x?}", self.args);

        self.vcpu.get_arch_vcpu().dump();

        Ok(HyperCallCode::HDebug as usize)
    }

    fn init_shim(&self) -> HyperCallResult {
        crate::libos::instance::init_shim()?;
        Ok(0)
    }

    fn create_instance(
        &self,
        instance_type: InstanceType,
        mapping_type: GuestMappingType,
        file_size: u64,
        shared_pages_base_gva: u64,
        shared_pages_num: u64,
    ) -> HyperCallResult {
        info!(
            "HCreateInstance type {:?}, mapping type {:?}, file size {} Bytes, shared_pages_base_gva {:#x} shared_pages_num {}",
            instance_type, mapping_type, file_size, shared_pages_base_gva, shared_pages_num
        );

        let instance_file = get_contents_from_shared_pages(
            file_size as _,
            shared_pages_base_gva as _,
            shared_pages_num as _,
            &self.vcpu,
            &self.vm,
        )?;

        crate::libos::instance::create_instance(instance_type, mapping_type, instance_file)
    }
}
