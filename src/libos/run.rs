use axerrno::AxResult;

use crate::libos::instance::InstanceRef;
use crate::vmm::VCpuRef;

/// TODO: maybe we tend to pin each vCpu on every physical CPU.
/// This function is the main routine for the vCPU task.
pub fn libos_vcpu_run(instance: InstanceRef, vcpu: VCpuRef) {
    let instance_id = instance.id();
    let vcpu_id = vcpu.id();

    info!("Instance[{}] Vcpu[{}] running", instance_id, vcpu_id);

    // Wait for the instance to be in the running state.
    while !instance.running() {
        info!(
            "Instance[{}] Vcpu[{}] waiting for instance to be running",
            instance_id, vcpu_id
        );
        core::hint::spin_loop();
    }

    loop {
        match instance.run_vcpu() {
            Ok(exit_reason) => {
                info!(
                    "Instance[{}] Vcpu[{}] exit reason: {:?}",
                    instance_id, vcpu_id, exit_reason
                );
                break;
                // Handle the exit reason here
            }
            Err(err) => {
                error!(
                    "Instance[{}] Vcpu[{}] run error: {:?}",
                    instance_id, vcpu_id, err
                );
                break;
            }
        }
    }
}
