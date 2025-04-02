use axerrno::AxResult;

use crate::libos::instance::InstanceRef;
use crate::vmm::VCpuRef;

pub fn libos_vcpu_run(instance: InstanceRef, vcpu: VCpuRef) {
    info!("Instance[{}] Vcpu[{}] run", instance.id(), vcpu.id());
}
