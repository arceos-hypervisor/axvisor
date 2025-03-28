mod config;
mod images;

#[cfg(feature = "irq")]
mod timer;
mod vcpus;
mod vm_list;

mod hypercall;

use std::os::arceos::api::task::{self, AxWaitQueueHandle};

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use crate::hal::{AxVCpuHalImpl, AxVMHalImpl};

#[cfg(feature = "irq")]
pub use timer::init_percpu as init_timer_percpu;

pub type VM = axvm::AxVM<AxVMHalImpl, AxVCpuHalImpl>;
pub type VMRef = axvm::AxVMRef<AxVMHalImpl, AxVCpuHalImpl>;

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
