#![no_std]
#![no_main]

#[macro_use]
extern crate log;

#[macro_use]
extern crate alloc;

extern crate axstd as std;
extern crate driver;

// extern crate axruntime;

mod logo;
mod task;

pub use axvisor_vmm::*;
pub use axvisor_shell::*;

#[unsafe(no_mangle)]
fn main() {
    logo::print_logo();

    info!("Starting virtualization...");
    // info!("Hardware support: {:?}", axvm::has_hardware_support());

    axvisor_vmm::init();
    axvisor_vmm::start_preconfigured_vms().unwrap();

    info!("[OK] Default guest initialized");
    axvisor_vmm::wait_for_all_vms_exit();
    info!("All guest VMs exited.");
    axvisor_shell::console_init();
}
