use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};

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
        match self.code {
            HyperCallCode::HIVC_ALLOC_MEM => {
                info!("VM[{}] HyperCall HIVC_ALLOC_MEM", self.vm.id());
				self.vm.map_region(gpa, hpa, size, flags);

				Ok(0)
            }
            _ => {
                warn!("Unsupported hypercall code: {:?}", self.code);
                return ax_err!(Unsupported);
            }
        }
    }
}
