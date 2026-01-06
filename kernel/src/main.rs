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
mod shell;
mod vmm;

pub use shell::*;
pub use vmm::*;

#[unsafe(no_mangle)]
fn main() {
    logo::print_logo();

    info!("Starting virtualization...");
    // info!("Hardware support: {:?}", axvm::has_hardware_support());

    vmm::init();
    vmm::start_preconfigured_vms().unwrap();

    info!("[OK] Default guest initialized");
    vmm::wait_for_all_vms_exit();
    info!("All guest VMs exited.");
    shell::console_init();
}
