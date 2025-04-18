mod config;
mod images;
mod mock;
pub mod timer;
mod vcpus;
mod vm_list;

use std::os::arceos::api::task::{self, AxWaitQueueHandle};

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use std::os::arceos::modules::axhal;
use std::os::arceos::modules::axtask;
use std::os::arceos::modules::axtask::TaskExtRef;

use axerrno::{AxResult, ax_err_type};

use crate::hal::{AxVCpuHalImpl, AxVMHalImpl};
pub use timer::init_percpu as init_timer_percpu;

pub type VM = axvm::AxVM<AxVMHalImpl, AxVCpuHalImpl>;
pub type VMRef = axvm::AxVMRef<AxVMHalImpl, AxVCpuHalImpl>;

pub type VCpuRef = axvm::AxVCpuRef<AxVCpuHalImpl>;

static VMM: AxWaitQueueHandle = AxWaitQueueHandle::new();

static RUNNING_VM_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn init() {
    // Initialize guest VM according to config file.
    config::init_guest_vms();

    // Setup vcpus, spawn axtask for primary VCpu.
    info!("Setting up vcpus...");
    for vm in vm_list::get_vm_list() {
        vcpus::setup_vm_primary_vcpu(vm);
    }
}

pub fn start() {
    info!("VMM starting, booting VMs...");
    for vm in vm_list::get_vm_list() {
        match vm.boot() {
            Ok(_) => {
                vcpus::notify_primary_vcpu(vm.id());
                RUNNING_VM_COUNT.fetch_add(1, Ordering::Release);
                info!("VM[{}] boot success", vm.id())
            }
            Err(err) => warn!("VM[{}] boot failed, error {:?}", vm.id(), err),
        }
    }

    // Do not exit until all VMs are stopped.
    task::ax_wait_queue_wait_until(&VMM, || RUNNING_VM_COUNT.load(Ordering::Acquire) == 0, None);
}

#[allow(unused_imports)]
pub use vcpus::{find_vcpu_task, with_vcpu_task};

/// Run a closure with the specified VM.
pub fn with_wm<T>(vm_id: usize, f: impl FnOnce(VMRef) -> T) -> Option<T> {
    let vm = vm_list::get_vm_by_id(vm_id)?;
    Some(f(vm))
}

/// Run a closure with the specified VM and vCPU.
pub fn with_vm_and_vcpu<T>(
    vm_id: usize,
    vcpu_id: usize,
    f: impl FnOnce(VMRef, VCpuRef) -> T,
) -> Option<T> {
    let vm = vm_list::get_vm_by_id(vm_id)?;
    let vcpu = vm.vcpu(vcpu_id)?;

    Some(f(vm, vcpu))
}

/// Run a closure with the specified VM and vCPU, with the guarantee that the closure will be
/// executed on the physical CPU where the vCPU is running, waiting, or queueing.
///
/// TODO: It seems necessary to disable scheduling when running the closure.
pub fn with_vm_and_vcpu_on_pcpu(
    vm_id: usize,
    vcpu_id: usize,
    f: impl FnOnce(VMRef, VCpuRef) + 'static,
) -> AxResult {
    // Disables preemption and IRQs to prevent the current task from being preempted or re-scheduled.
    let guard = kernel_guard::NoPreemptIrqSave::new();

    let current_vm = axtask::current().task_ext().vm.id();
    let current_vcpu = axtask::current().task_ext().vcpu.id();

    // The target vCPU is the current task, execute the closure directly.
    if current_vm == vm_id && current_vcpu == vcpu_id {
        with_vm_and_vcpu(vm_id, vcpu_id, f).unwrap(); // unwrap is safe here
        return Ok(());
    }

    // The target vCPU is not the current task, send an IPI to the target physical CPU.
    drop(guard);

    let pcpu_id = vcpus::with_vcpu_task(vm_id, vcpu_id, |task| task.cpu_id())
        .ok_or_else(|| ax_err_type!(NotFound))?;

    use std::os::arceos::modules::axipi;
    Ok(axipi::send_ipi_event_to_one(pcpu_id as usize, move || {
        with_vm_and_vcpu_on_pcpu(vm_id, vcpu_id, f);
    }))
}
