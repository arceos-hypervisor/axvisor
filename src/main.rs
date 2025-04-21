#![no_std]
#![no_main]

use std::println;

#[macro_use]
extern crate log;
#[macro_use]
extern crate alloc;

extern crate axstd as std;

mod hal;
mod task;
mod vmm;

const LOGO: &'static str = r#"
       d8888            888     888  d8b
      d88888            888     888  Y8P
     d88P888            888     888
    d88P 888  888  888  Y88b   d88P  888  .d8888b    .d88b.   888d888
   d88P  888  `Y8bd8P'   Y88b d88P   888  88K       d88""88b  888P"
  d88P   888    X88K      Y88o88P    888  "Y8888b.  888  888  888
 d8888888888  .d8""8b.     Y888P     888       X88  Y88..88P  888
d88P     888  888  888      Y8P      888   88888P'   "Y88P"   888
"#;

fn print_logo() {
    println!();
    println!("{}", LOGO);
    println!();
}

#[unsafe(no_mangle)]
fn main() {
    print_logo();

    info!("Starting virtualization...");

    info!("Hardware support: {:?}", axvm::has_hardware_support());

    hal::enable_virtualization();

    vmm::init();

    vmm::start();

    info!("VMM shutdown");
}
