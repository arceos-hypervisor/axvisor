#[axvisor_api::api_mod_impl(axvisor_api::arch)]
mod arch_api_impl {
    extern fn hardware_inject_virtual_interrupt(irq: axvisor_api::vmm::InterruptVector) {
        crate::hal::arch::inject_interrupt(irq as _);
    }

    extern fn read_vgicd_typer() -> u32 {
        let mut gic = rdrive::get_one::<rdrive::driver::Intc>()
            .expect("Failed to get GIC driver")
            .lock()
            .unwrap();
        if let Some(gic) = gic.typed_mut::<arm_gic_driver::v2::Gic>() {
            return gic.typer_raw();
        }

        if let Some(gic) = gic.typed_mut::<arm_gic_driver::v3::Gic>() {
            // Use the GICv3 driver to read the typer register
            return gic.typer_raw();
        }

        // use axstd::os::arceos::modules::axhal::irq::MyVgic;
        // MyVgic::get_gicd().lock().get_typer()

        // use memory_addr::pa;
        // use std::os::arceos::modules::{axconfig, axhal};

        unimplemented!();
        // let typer_phys_addr = axconfig::devices::GICD_PADDR + 0x4;
        // let typer_virt_addr = axhal::mem::phys_to_virt(pa!(typer_phys_addr));

        // unsafe { core::ptr::read_volatile(typer_virt_addr.as_ptr_of::<u32>()) }
    }

    extern fn read_vgicd_iidr() -> u32 {
        // use axstd::os::arceos::modules::axhal::irq::MyVgic;
        // MyVgic::get_gicd().lock().get_iidr()
        let mut gic = rdrive::get_one::<rdrive::driver::Intc>()
            .expect("Failed to get GIC driver")
            .lock()
            .unwrap();
        if let Some(gic) = gic.typed_mut::<arm_gic_driver::v2::Gic>() {
            return gic.iidr_raw();
        }

        if let Some(gic) = gic.typed_mut::<arm_gic_driver::v3::Gic>() {
            // Use the GICv3 driver to read the typer register
            return gic.iidr_raw();
        }

        unimplemented!()
    }

    extern fn get_host_gicd_base() -> memory_addr::PhysAddr {
        // use std::os::arceos::api::config;
        // unimplemented!();
        // config::devices::GICD_PADDR.into()
        0x800_0000.into()
    }

    extern fn get_host_gicr_base() -> memory_addr::PhysAddr {
        // use std::os::arceos::api::config;
        // unimplemented!();
        // config::devices::GICR_PADDR.into()
        // TODO parse from dtb
        0x80a_0000.into()
    }
}
