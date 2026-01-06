#![no_std]

#[macro_use]
extern crate log;

#[macro_use]
extern crate alloc;
extern crate axstd;

// mod hvc;
// mod ivc;

pub mod config;
pub mod images;
// pub mod timer;
pub mod vm_list;

use axvm::{AxVMConfig, VmId};

/// Initialize the VMM.
///
/// This function creates the VM structures and sets up the primary VCpu for each VM.
pub fn init() {
    info!("Initializing VMM...");
    axvm::enable_viretualization().unwrap();
}

pub fn start_preconfigured_vms() -> anyhow::Result<()> {
    // Initialize guest VM according to config file.
    for config in config::get_guest_prelude_vmconfig()? {
        let vm_config = config::build_vmconfig(config)?;
        start_vm(vm_config)?;
    }
    Ok(())
}

pub fn start_vm(config: AxVMConfig) -> anyhow::Result<VmId> {
    debug!("Starting guest VM `{}`", config.name());
    let vm = axvm::Vm::new(config)?;
    let vm = vm_list::push_vm(vm);
    vm.boot()?;
    Ok(vm.id())
}

pub fn wait_for_all_vms_exit() {
    let ls = vm_list::get_vm_list();
    for vm in ls.iter() {
        vm.wait().unwrap();
    }
}
