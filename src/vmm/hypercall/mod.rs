use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos;

use arceos::modules::axhal;

use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::{AxArchVCpu, AxVcpuAccessGuestState};

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
        warn!("Hypercall: {:?} args: {:#x?}", self.code, self.args);

        if self.vcpu.get_arch_vcpu().guest_is_privileged() {
            self.execute_privileged()
        } else {
            self.execute_unprivileged()
        }
    }

    fn execute_privileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HypervisorDisable => self.hypervisor_disable(),
            _ => {
                unimplemented!()
            }
        }
    }

    fn execute_unprivileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HDebug => self.debug(),
            HyperCallCode::HCreateInstance => self.create_instance(
                self.args[0],
                self.args[1],
                self.args[2],
                self.args[3],
                self.args[4],
                self.args[5],
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
        Ok(HyperCallCode::HDebug as usize)
    }

    fn create_instance(
        &self,
        id: u64,
        memory_region_cnt: u64,
        memory_cfg_pages_base_gva: u64,
        memory_cfg_pages_count: u64,
        entry: u64,
        mapping_type: u64,
    ) -> HyperCallResult {
        info!(
            "HCreateInstance iid:{} mm_cnt:{} base_gva:{:#x} pages_cnt: {} entry {:#x}",
            id, memory_region_cnt, memory_cfg_pages_base_gva, memory_cfg_pages_count, entry
        );

        let process_regions = crate::libos::def::process_elf_memory_regions(
            memory_region_cnt as _,
            memory_cfg_pages_base_gva as _,
            memory_cfg_pages_count as _,
            &self.vcpu,
            &self.vm,
        )?;

        // Currently we just construct user process context from current vcpu context.
        // It can be regarded as a duplicate of the context of current Linux process which trigger the `HCreateInstance` hypercall.
        let mut ctx = axhal::get_linux_context_list()[axhal::cpu::this_cpu_id() as usize].clone();
        self.vcpu.get_arch_vcpu().load_context(&mut ctx)?;

        // Set the entry point (`rip`) of the new process as entry parsed from the ELF file.
        ctx.rip = entry as u64;

        crate::libos::instance::create_instance(
            id as usize,
            process_regions,
            ctx,
            mapping_type.into(),
        )?;

        Ok(0)
    }
}
