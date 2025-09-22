pub(crate) mod config;
mod images;

#[cfg(feature = "irq")]
mod timer;
mod vcpus;
mod vm_list;

mod hypercall;
pub(crate) mod ivc;

use std::os::arceos::api::task::{self, AxWaitQueueHandle};

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use axerrno::{AxResult, ax_err_type};

use crate::hal::{AxVCpuHalImpl, AxVMHalImpl};

#[cfg(feature = "irq")]
pub use timer::init_percpu as init_timer_percpu;

pub type VM = axvm::AxVM<AxVMHalImpl, AxVCpuHalImpl>;
pub type VMRef = axvm::AxVMRef<AxVMHalImpl, AxVCpuHalImpl>;

pub type VCpu = axvm::VCpu<AxVCpuHalImpl>;
pub type VCpuRef = axvm::AxVCpuRef<AxVCpuHalImpl>;

static VMM: AxWaitQueueHandle = AxWaitQueueHandle::new();

static RUNNING_VM_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn init() {
    config::init_host_vm();
    // Initialize guest VM according to config file.
    config::init_guest_vms();

    // Setup vcpus, spawn axtask for primary VCpu.
    info!("Setting up vcpus...");

    vm_list::manipulate_each_vm(|vm| {
        vcpus::setup_vm_cpu(vm);
    });
}

pub fn start() {
    info!("VMM starting, booting VMs...");
    vm_list::manipulate_each_vm(|vm| {
        let _ = vm
            .boot()
            .inspect(|_| {
                vcpus::boot_vm_cpu(&vm);
                RUNNING_VM_COUNT.fetch_add(1, Ordering::Release);
                info!("VM[{}] boot success", vm.id());
            })
            .inspect_err(|err| {
                warn!("VM[{}] boot failed, error {:?}", vm.id(), err);
            });
    });

    // Do not exit until all VMs are stopped.
    task::ax_wait_queue_wait_until(&VMM, || RUNNING_VM_COUNT.load(Ordering::Acquire) == 0, None);
}

pub fn boot_vm(vm_id: usize) -> AxResult {
    let vm = vm_list::get_vm_by_id(vm_id).ok_or_else(|| {
        warn!("VM with ID {} not found", vm_id);
        ax_err_type!(InvalidInput, "VM not found")
    })?;

    // First, setup VCPUs for the VM.
    vcpus::setup_vm_cpu(vm.clone());

    info!("Booting VM [{}]", vm.id());

    vm.boot()
        .inspect(|_| {
            vcpus::boot_vm_cpu(&vm);
            RUNNING_VM_COUNT.fetch_add(1, Ordering::Release);
            info!("VM[{}] boot success", vm.id());
        })
        .inspect_err(|err| {
            warn!("VM[{}] boot failed, error {:?}", vm.id(), err);
        })
}

use std::os::arceos::modules::axtask;

use axtask::TaskExtRef;

use crate::task_ext::TaskExtType;

/// The main routine for vCPU task.
/// This function is the entry point for the vCPU tasks, which are spawned for each vCPU of a VM.
///
/// When the vCPU first starts running, it waits for the VM to be in the running state.
/// It then enters a loop where it runs the vCPU and handles the various exit reasons.
pub fn vcpu_run() {
    let curr = axtask::current();

    match &curr.task_ext().ext {
        TaskExtType::VM(vm) => {
            crate::vmm::vcpus::vm_vcpu_run(vm.clone(), curr.task_ext().vcpu.clone());
        }
        TaskExtType::LibOS => {
            crate::libos::libos_vcpu_run(curr.task_ext().vcpu.clone());
        }
    };
}
