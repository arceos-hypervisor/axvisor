#![no_std]
#![no_main]

#[macro_use]
extern crate log;

#[macro_use]
extern crate alloc;

extern crate axstd as std;

#[cfg(target_arch = "aarch64")]
extern crate axplat_aarch64_generic;

#[cfg(target_arch = "x86_64")]
extern crate axplat_x86_qemu_q35;

#[cfg(feature = "fs")]
mod shell;

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

    info!("[OK] Default guest initialized");

    #[cfg(feature = "fs")]
    shell::console_init();
}
