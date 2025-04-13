use core::sync::atomic::AtomicBool;
use std::collections::btree_map::BTreeMap;
use std::os::arceos::modules::axhal::cpu::this_cpu_id;
use std::os::arceos::modules::{axconfig, axtask};
use std::sync::Arc;

use lazyinit::LazyInit;

use axconfig::SMP;
use axtask::{AxCpuMask, AxTaskRef, TaskInner};

use crate::libos::instance::InstanceRef;
use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::{VCpu, VCpuRef};

use instance::InstanceRef;

const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

#[derive(Default)]
struct LibOSPerCpu {
    vcpu: LazyInit<VCpuRef>,
    running: AtomicBool,
}

static LIBOS_PERCPU: [LibOSPerCpu; SMP] = [LibOSPerCpu::default(); SMP];

pub fn init_instance_percore_task(cpu_id: usize, vcpu: VCpuRef) {
    let mut vcpu_task: TaskInner = TaskInner::new(
        crate::vmm::vcpu_run,
        format!("ICore [{}]", cpu_id),
        KERNEL_STACK_SIZE,
    );

    vcpu_task.init_task_ext(TaskExt::new(TaskExtType::LibOS, vcpu));

    vcpu_task.set_cpumask(AxCpuMask::one_shot(cpu_id));

    axtask::spawn_task(vcpu_task).expect("Failed to spawn vcpu task");
}

/// TODO: maybe we tend to pin each vCpu on every physical CPU.
/// This function is the main routine for the vCPU task.
pub fn libos_vcpu_run(vcpu: VCpuRef) {
    let vcpu_id = vcpu.id();
    let cpu_id = this_cpu_id();

    info!(
        "Instance task on Core[{}] running, VCPU id {}",
        cpu_id, vcpu_id
    );

    let percpu = &LIBOS_PERCPU[cpu_id];

    // Wait for the instance to be in the running state.
    while !percpu.running.load(core::sync::atomic::Ordering::SeqCst) {
        info!(
            "Vcpu[{}] waiting for instance to be running",
            instance_id, vcpu_id
        );
        core::hint::spin_loop();
    }

    loop {
        match vcpu.run() {
            Ok(exit_reason) => {
                info!("Vcpu[{}] exit reason: {:?}", vcpu_id, exit_reason);
                break;
                // Handle the exit reason here
            }
            Err(err) => {
                error!("Vcpu[{}] run error: {:?}", vcpu_id, err);
                break;
            }
        }
    }
}
