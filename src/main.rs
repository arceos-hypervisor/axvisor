#![no_std]
#![no_main]

use std::os::arceos::modules::axlog;

#[macro_use]
extern crate log;
#[macro_use]
extern crate alloc;

extern crate axstd as std;

mod hal;
mod task_ext;
mod vmm;

mod region;

mod libos;

const LOGO: &str = r"
 _____                  _   _              ___  ____  
| ____|__ _ _   _  __ _| |_(_) ___  _ __  / _ \/ ___| 
|  _| / _` | | | |/ _` | __| |/ _ \| '_ \| | | \___ \ 
| |__| (_| | |_| | (_| | |_| | (_) | | | | |_| |___) |
|_____\__, |\__,_|\__,_|\__|_|\___/|_| |_|\___/|____/ 
         |_|                                              
             ___   ____ ___   ______
            |__ \ / __ \__ \ / ____/
            __/ // / / /_/ //___ \  
           / __// /_/ / __/____/ /  
          /____/\____/____/_____/   
";

fn dump_equation_defs() {
    info!("EquationDefs");

    equation_defs::dump_addrs();
}

#[unsafe(no_mangle)]
fn main() {
    axlog::ax_println!("{}", LOGO);

    dump_equation_defs();

    info!("Starting virtualization...");

    info!("Hardware support: {:?}", axvm::has_hardware_support());

    hal::enable_virtualization();

    vmm::init();

    vmm::start();

    info!("VMM shutdown");
}
