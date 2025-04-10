use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos;

use arceos::modules::axhal;

use bit_field::BitField;
use numeric_enum_macro::numeric_enum;

use axerrno::{AxResult, ax_err, ax_err_type};
use axvcpu::{AxArchVCpu, AxVcpuAccessGuestState};

use crate::vmm::{VCpu, VCpuRef, VMRef};

const HYPER_CALL_CODE_PRIVILEGED_MASK: u32 = 0xc000_0000;

numeric_enum! {
    #[repr(u32)]
    #[derive(Eq, PartialEq, Copy, Clone)]
    pub enum HyperCallCode {
        /// Disable the hypervisor.
        HypervisorDisable = 0,
        /// Prepare to disable the hypervisor, map the hypervisor memory to the guest.
        HyperVisorPrepareDisable = 1,
        HDebug = HYPER_CALL_CODE_PRIVILEGED_MASK | 0,
        HCreateInstance = HYPER_CALL_CODE_PRIVILEGED_MASK | 1,
        HCreateInitProcess = HYPER_CALL_CODE_PRIVILEGED_MASK | 2,
        HMMAP = HYPER_CALL_CODE_PRIVILEGED_MASK | 3,
        HClone = HYPER_CALL_CODE_PRIVILEGED_MASK | 4,
    }
}

impl core::fmt::Debug for HyperCallCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "(")?;
        match self {
            HyperCallCode::HypervisorDisable => write!(f, "HypervisorDisable {:#x}", *self as u32),
            HyperCallCode::HyperVisorPrepareDisable => {
                write!(f, "HyperVisorPrepareDisable {:#x}", *self as u32)
            }
            HyperCallCode::HDebug => write!(f, "HDebug {:#x}", *self as u32),
            HyperCallCode::HCreateInstance => write!(f, "HCreateInstance {:#x}", *self as u32),
            HyperCallCode::HCreateInitProcess => {
                write!(f, "HCreateInitProcess {:#x}", *self as u32)
            }
            HyperCallCode::HMMAP => write!(f, "HMMAP {:#x}", *self as u32),
            HyperCallCode::HClone => write!(f, "HClone {:#x}", *self as u32),
        }?;
        write!(f, ")")
    }
}

impl HyperCallCode {
    fn is_privileged(self) -> bool {
        (self as u32).get_bits(30..32) == 0
    }
}

pub type HyperCallResult = AxResult<usize>;

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
            HyperCallCode::HCreateInstance => {
                self.create_instance(self.args[0], self.args[1], self.args[2], self.args[3])
            }
            HyperCallCode::HCreateInitProcess => {
                self.create_init_process(self.args[0], self.args[1])
            }
            HyperCallCode::HClone => self.clone(),
            HyperCallCode::HMMAP => {
                self.mmap(self.args[0], self.args[1], self.args[2], self.args[3])
            }
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
    ) -> HyperCallResult {
        info!(
            "HCreateInstance iid:{} mm_cnt:{} base_gva:{:#x} pages_cnt: {}",
            id, memory_region_cnt, memory_cfg_pages_base_gva, memory_cfg_pages_count
        );

        let process_regions = crate::libos::def::process_libos_memory_regions(
            memory_region_cnt as _,
            memory_cfg_pages_base_gva as _,
            memory_cfg_pages_count as _,
            &self.vcpu,
            &self.vm,
        );

        let mut host_ctx =
            axhal::get_linux_context_list()[axhal::cpu::this_cpu_id() as usize].clone();

        self.vcpu.get_arch_vcpu().load_host(&mut host_ctx)?;

        let instance_cpu_mask = crate::vmm::config::alloc_instance_cpus_bitmap(1);

        debug!(
            "Generate instance {} vcpu, cpu mask: {:#x}, host ctx: {:x?}",
            id, instance_cpu_mask, host_ctx
        );

        let instance_vcpu = Arc::new(VCpu::new_host(
            id as _,
            host_ctx,
            Some(instance_cpu_mask as _),
        )?);
        crate::libos::instance::create_instance(id as usize, process_regions, instance_vcpu)?;

        Ok(0)
    }

    fn create_init_process(&self, iid: u64, pid: u64) -> HyperCallResult {
        info!("HCreateInitProcess iid:{} pid:{}", iid, pid);
        crate::libos::instance::manipulate_instance(iid as _, |instance| {
            instance.create_init_process(pid as usize)
        })?;
        Ok(0)
    }

    fn clone(&self) -> HyperCallResult {
        info!("HClone");
        Ok(0)
    }

    fn mmap(&self, addr: u64, len: u64, prot: u64, flags: u64) -> HyperCallResult {
        info!(
            "HMMAP addr:{:#x} len:{} prot:{} flags:{}",
            addr, len, prot, flags
        );
        Ok(0)
    }
}
