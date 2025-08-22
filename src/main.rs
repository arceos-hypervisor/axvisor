#![no_std]
#![no_main]

#[macro_use]
extern crate log;
#[macro_use]
extern crate alloc;

extern crate axstd as std;

#[cfg(feature = "plat-aarch64-generic")]
extern crate axplat_aarch64_generic;
#[cfg(feature = "plat-x86-qemu-q35")]
extern crate axplat_x86_qemu_q35;

mod hal;
mod logo;
mod task;
mod vmm;

#[unsafe(no_mangle)]
fn main() {
    logo::print_logo();

    info!("Starting virtualization...");
    info!("Hardware support: {:?}", axvm::has_hardware_support());
    hal::enable_virtualization();

    vmm::init();
    vmm::start();

    info!("VMM shutdown");
}
