pub mod def;
pub mod instance;
pub mod process;
mod run;

pub use run::libos_vcpu_run;

use std::os::arceos::modules::axtask;

use axtask::{AxCpuMask, AxTaskRef, TaskInner};

use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::VCpuRef;

use instance::InstanceRef;

const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

fn alloc_vcpu_task(insance: InstanceRef, vcpu: VCpuRef) -> AxTaskRef {
    info!(
        "Spawning task for Instance[{}] Vcpu[{}]",
        insance.id(),
        vcpu.id()
    );
    let mut vcpu_task: TaskInner = TaskInner::new(
        crate::vmm::vcpu_run,
        format!("Instance[{}]-VCpu[{}]", insance.id(), vcpu.id()),
        KERNEL_STACK_SIZE,
    );

    if let Some(phys_cpu_set) = vcpu.phys_cpu_set() {
        vcpu_task.set_cpumask(AxCpuMask::from_raw_bits(phys_cpu_set));
    }

    vcpu_task.init_task_ext(TaskExt::new(TaskExtType::LibOS(insance), vcpu));

    info!(
        "Vcpu task {} created {:?}",
        vcpu_task.id_name(),
        vcpu_task.cpumask()
    );
    axtask::spawn_task(vcpu_task)
}
