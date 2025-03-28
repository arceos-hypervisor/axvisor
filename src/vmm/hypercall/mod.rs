use bit_field::BitField;
use numeric_enum_macro::numeric_enum;

use axerrno::{AxResult, ax_err, ax_err_type};
use axvcpu::AxVcpuAccessGuestState;

use crate::vmm::{VCpuRef, VMRef};

const HYPER_CALL_CODE_PRIVILEGED_MASK: u32 = 0xc000_0000;

numeric_enum! {
    #[repr(u32)]
    #[derive(Eq, PartialEq, Copy, Clone)]
    pub enum HyperCallCode {
        HypervisorDisable = 0,
        HDebug = HYPER_CALL_CODE_PRIVILEGED_MASK | 0,
        HCreateInstance = HYPER_CALL_CODE_PRIVILEGED_MASK | 1,
        HClone = HYPER_CALL_CODE_PRIVILEGED_MASK | 2,
        HMMAP = HYPER_CALL_CODE_PRIVILEGED_MASK | 3,
    }
}

impl core::fmt::Debug for HyperCallCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "(")?;
        match self {
            HyperCallCode::HypervisorDisable => write!(f, "HypervisorDisable {:#x}", *self as u32),
            HyperCallCode::HDebug => write!(f, "HDebug {:#x}", *self as u32),
            HyperCallCode::HCreateInstance => write!(f, "HCreateInstance {:#x}", *self as u32),
            HyperCallCode::HClone => write!(f, "HClone {:#x}", *self as u32),
            HyperCallCode::HMMAP => write!(f, "HMMAP {:#x}", *self as u32),
        }?;
        write!(f, ")")
    }
}

impl HyperCallCode {
    fn is_privileged(self) -> bool {
        (self as u32).get_bits(30..32) == 0
    }
}

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

    pub fn execute(&self) -> AxResult {
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
        debug!("Hypercall: {:?} args: {:?}", self.code, self.args);

        if self.vcpu.get_arch_vcpu().guest_is_privileged() {
            self.execute_privileged()
        } else {
            self.execute_unprivileged()
        }
    }

    fn execute_privileged(&self) -> AxResult {
        match self.code {
            HyperCallCode::HypervisorDisable => self.hypervisor_disable(),
            _ => {
                unimplemented!()
            }
        }
    }

    fn execute_unprivileged(&self) -> AxResult {
        match self.code {
            HyperCallCode::HDebug => self.debug(),
            HyperCallCode::HCreateInstance => {
                self.create_instance(self.args[0], self.args[1], self.args[2], self.args[3])
            }
            HyperCallCode::HClone => self.clone(),
            HyperCallCode::HMMAP => self.mmap(),
            _ => {
                unimplemented!();
            }
        }
    }
}

impl HyperCall {
    fn hypervisor_disable(&self) -> AxResult {
        info!("HypervisorDisable");
        Ok(())
    }

    fn debug(&self) -> AxResult {
        info!("HDebug {:#x?}", self.args);
        Ok(())
    }

    fn create_instance(
        &self,
        id: u64,
        memory_region_cnt: u64,
        memory_cfg_pages_base_gva: u64,
        memory_cfg_pages_count: u64,
    ) -> AxResult {
        info!(
            "HCreateInstance pid:{} mm_cnt:{} base_gva:{:#x} pages_cnt: {}",
            id, memory_region_cnt, memory_cfg_pages_base_gva, memory_cfg_pages_count
        );

        crate::libos::def::process_libos_memory_regions(
            memory_region_cnt as _,
            memory_cfg_pages_base_gva as _,
            memory_cfg_pages_count as _,
            &self.vcpu,
            &self.vm,
        );

        Ok(())
    }

    fn clone(&self) -> AxResult {
        info!("HClone");
        Ok(())
    }

    fn mmap(&self) -> AxResult {
        info!("HMMAP");
        Ok(())
    }
}
