use std::os::arceos::modules::axhal::cpu::this_cpu_id;
use std::os::arceos::modules::{axconfig, axhal, axtask};

use axaddrspace::GuestVirtAddr;
use lazyinit::LazyInit;
use page_table_multiarch::PagingHandler;

use axconfig::SMP;
use axtask::{AxCpuMask, TaskInner};

use crate::libos::instance::get_instances_by_id;
use crate::task_ext::{TaskExt, TaskExtType};
use crate::vmm::VCpuRef;
use crate::vmm::config::get_instance_cpus_mask;

const KERNEL_STACK_SIZE: usize = 0x40000; // 256 KiB

struct LibOSPerCpu {
    vcpu: VCpuRef,
    // running: AtomicBool,
}

#[percpu::def_percpu]
static LIBOS_PERCPU: LazyInit<LibOSPerCpu> = LazyInit::new();

pub fn init_instance_percore_task(cpu_id: usize, vcpu: VCpuRef) {
    assert!(cpu_id < SMP, "Invalid CPU ID: {}", cpu_id);
    if !get_instance_cpus_mask().get(cpu_id) {
        warn!(
            "CPU {} is not in the instance CPU mask, skipping task creation",
            cpu_id
        );
        return;
    }

    let remote_percpu = unsafe { LIBOS_PERCPU.remote_ref_raw(cpu_id) };

    if remote_percpu.is_inited() {
        warn!("PerCPU data for CPU {} is already initialized", cpu_id);
        return;
    }

    remote_percpu.init_once(LibOSPerCpu { vcpu: vcpu.clone() });

    let mut vcpu_task: TaskInner = TaskInner::new(
        crate::vmm::vcpu_run,
        format!("ICore [{}]", cpu_id),
        KERNEL_STACK_SIZE,
    );

    vcpu_task.init_task_ext(TaskExt::new(TaskExtType::LibOS, vcpu));

    vcpu_task.set_cpumask(AxCpuMask::one_shot(cpu_id));

    axtask::spawn_task(vcpu_task);
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

    let _percpu = unsafe { LIBOS_PERCPU.current_ref_raw() };

    vcpu.bind()
        .inspect_err(|err| {
            error!("Vcpu[{}] bind error: {:?}", vcpu_id, err);
            return;
        })
        .unwrap();
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
    vcpu.unbind()
        .inspect_err(|err| {
            error!("Vcpu[{}] unbind error: {:?}", vcpu_id, err);
        })
        .unwrap();
}
