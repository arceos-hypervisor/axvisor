// mod hvc;
// mod ivc;

pub mod config;
pub mod images;
// pub mod timer;
pub mod vm_list;

// #[cfg(target_arch = "aarch64")]
// pub mod fdt;

use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos::api::task::AxWaitQueueHandle;

use axvm::AxVMConfig;
use rdrive::get_list;

// pub use timer::init_percpu as init_timer_percpu;

static VMM: AxWaitQueueHandle = AxWaitQueueHandle::new();

/// The number of running VMs. This is used to determine when to exit the VMM.
static RUNNING_VM_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Initialize the VMM.
///
/// This function creates the VM structures and sets up the primary VCpu for each VM.
pub fn init() {
    info!("Initializing VMM...");

    axvm::enable_viretualization().unwrap();
}

pub fn get_running_vm_count() -> usize {
    RUNNING_VM_COUNT.load(Ordering::Acquire)
}

pub fn add_running_vm_count(count: usize) {
    RUNNING_VM_COUNT.fetch_add(count, Ordering::Release);
}

pub fn sub_running_vm_count(count: usize) {
    RUNNING_VM_COUNT.fetch_sub(count, Ordering::Release);
}

pub fn start_preconfigured_vms() -> anyhow::Result<()> {
    // Initialize guest VM according to config file.
    for config in config::get_guest_prelude_vmconfig()? {
        let vm_config = config::build_vmconfig(config)?;
        start_vm(vm_config)?;
    }
    Ok(())
}

pub fn start_vm(config: AxVMConfig) -> anyhow::Result<()> {
    debug!("Starting guest VM `{}`", config.name());
    let vm = axvm::Vm::new(config)?;
    let vm = vm_list::push_vm(vm);
    vm.boot()?;
    Ok(())
}

pub fn wait_for_all_vms_exit() {
    let ls = vm_list::get_vm_list();
    for vm in ls.iter() {
        vm.wait().unwrap();
    }
}
