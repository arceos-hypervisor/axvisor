use core::alloc::Layout;

use alloc::string::ToString;

use axaddrspace::GuestPhysAddr;
use axvm::{
    VMMemoryRegion,
    config::{AxVMConfig, AxVMCrateConfig, PassThroughDeviceConfig, VmMemMappingType},
};
use memory_addr::MemoryAddr;

use crate::vmm::{VM, images::ImageLoader, vm_list::push_vm};

#[allow(clippy::module_inception)]
pub mod config {
    use alloc::vec::Vec;

    /// Default static VM configs. Used when no VM config is provided.
    #[allow(dead_code)]
    pub fn default_static_vm_configs() -> Vec<&'static str> {
        vec![
            #[cfg(target_arch = "x86_64")]
            core::include_str!("../../configs/vms/nimbos-x86_64-qemu-smp1.toml"),
            #[cfg(target_arch = "aarch64")]
            core::include_str!("../../configs/vms/nimbos-aarch64-qemu-smp1.toml"),
            #[cfg(target_arch = "riscv64")]
            core::include_str!("../../configs/vms/nimbos-riscv64-qemu-smp1.toml"),
        ]
    }

    include!(concat!(env!("OUT_DIR"), "/vm_configs.rs"));
}

pub fn get_vm_dtb(vm_cfg: &AxVMConfig) -> Option<&'static [u8]> {
    let vm_imags = config::get_memory_images()
        .iter()
        .find(|&v| v.id == vm_cfg.id())?;
    // .expect("VM images is missed, Perhaps add `VM_CONFIGS=PATH/CONFIGS/FILE` command.");
    vm_imags.dtb
}

pub fn parse_vm_dtb(vm_cfg: &mut AxVMConfig, dtb: &[u8]) {
    use fdt_parser::{Fdt, Status};

    let fdt = Fdt::from_bytes(dtb)
        .expect("Failed to parse DTB image, perhaps the DTB is invalid or corrupted");

    for reserved in fdt.reserved_memory() {
        warn!("Find reserved memory: {:?}", reserved.name());
    }

    for mem_reserved in fdt.memory_reservation_block() {
        warn!("Find memory reservation block: {:?}", mem_reserved);
    }

    for node in fdt.all_nodes() {
        trace!("DTB node: {:?}", node.name());
        let name = node.name();
        if name.starts_with("memory") {
            // Skip the memory node, as we handle memory regions separately.
            continue;
        }

        if let Some(status) = node.status()
            && status == Status::Disabled
        {
            // Skip disabled nodes
            trace!("DTB node: {} is disabled", name);
            // continue;
        }

        // Skip the interrupt controller, as we will use vGIC
        // TODO: filter with compatible property and parse its phandle from DT; maybe needs a second pass?
        const GIC_PHANDLE: usize = 1;
        if name.starts_with("interrupt-controller")
            || name.starts_with("intc")
            || name.starts_with("its")
        {
            info!("skipping node {} to use vGIC", name);
            continue;
        }

        // Collect all GIC_SPI interrupts and add them to vGIC
        if let Some(interrupts) = node.interrupts() {
            // TODO: skip non-GIC interrupt
            if let Some(parent) = node.interrupt_parent() {
                trace!("node: {}, intr parent: {}", name, parent.node.name());
                if let Some(phandle) = parent.node.phandle() {
                    if phandle.as_usize() != GIC_PHANDLE {
                        warn!(
                            "node: {}, intr parent: {}, phandle: 0x{:x} is not GIC!",
                            name,
                            parent.node.name(),
                            phandle.as_usize()
                        );
                    }
                } else {
                    warn!(
                        "node: {}, intr parent: {} no phandle!",
                        name,
                        parent.node.name(),
                    );
                }
            } else {
                warn!("node: {} no interrupt parent!", name);
            }

            trace!("node: {} interrupts:", name);

            for interrupt in interrupts {
                // <GIC_SPI/GIC_PPI, IRQn, trigger_mode>
                for (k, v) in interrupt.enumerate() {
                    match k {
                        0 => {
                            if v == 0 {
                                trace!("node: {}, GIC_SPI", name);
                            } else {
                                warn!(
                                    "node: {}, intr type: {}, not GIC_SPI, not supported!",
                                    name, v
                                );
                                break;
                            }
                        }
                        1 => {
                            trace!("node: {}, interrupt id: 0x{:x}", name, v);
                            vm_cfg.add_pass_through_spi(v);
                        }
                        2 => {
                            trace!("node: {}, interrupt mode: 0x{:x}", name, v);
                        }
                        _ => {
                            warn!("unknown interrupt property {}:0x{:x}", k, v)
                        }
                    }
                }
            }
        }

        if let Some(regs) = node.reg() {
            for reg in regs {
                if reg.address < 0x1000 {
                    // Skip registers with address less than 0x10000.
                    trace!(
                        "Skipping DTB node {} with register address {:#x} < 0x10000",
                        node.name(),
                        reg.address
                    );
                    continue;
                }

                if let Some(size) = reg.size {
                    let start = reg.address as usize;
                    // let end = start + size;
                    // if vm_cfg.contains_memory_range(&(start..end)) {
                    //     trace!(
                    //         "Skipping DTB node {} with register address {:#x} and size {:#x} as it overlaps with existing memory regions",
                    //         node.name(),
                    //         reg.address,
                    //         size
                    //     );
                    //     continue;
                    // }

                    let pt_dev = PassThroughDeviceConfig {
                        name: node.name().to_string(),
                        base_gpa: start,
                        base_hpa: start,
                        length: size as _,
                        irq_id: 0,
                    };
                    trace!("Adding {:x?}", pt_dev);
                    vm_cfg.add_pass_through_device(pt_dev);
                }
            }
        }
    }

    vm_cfg.add_pass_through_device(PassThroughDeviceConfig {
        name: "Fake Node".to_string(),
        base_gpa: 0x0,
        base_hpa: 0x0,
        length: 0x20_0000,
        irq_id: 0,
    });
}

pub fn init_guest_vms() {
    let gvm_raw_configs = config::static_vm_configs();

    for raw_cfg_str in gvm_raw_configs {
        let vm_create_config =
            AxVMCrateConfig::from_toml(raw_cfg_str).expect("Failed to resolve VM config");

        if let Some(linux) = super::images::get_image_header(&vm_create_config) {
            debug!(
                "VM[{}] Linux header: {:#x?}",
                vm_create_config.base.id, linux
            );
        }

        let mut vm_config = AxVMConfig::from(vm_create_config.clone());

        // Overlay VM config with the given DTB.
        if let Some(dtb) = get_vm_dtb(&vm_config) {
            parse_vm_dtb(&mut vm_config, dtb);
        } else {
            warn!(
                "VM[{}] DTB not found in memory, skipping...",
                vm_config.id()
            );
        }

        info!("Creating VM[{}] {:?}", vm_config.id(), vm_config.name());

        // Create VM.
        let vm = VM::new(vm_config).expect("Failed to create VM");
        push_vm(vm.clone());

        vm_alloc_memorys(&vm_create_config, &vm);

        let main_mem = vm
            .memory_regions()
            .first()
            .cloned()
            .expect("VM must have at least one memory region");

        config_guest_address(&vm, &main_mem);

        // Load corresponding images for VM.
        info!("VM[{}] created success, loading images...", vm.id());

        let mut loader = ImageLoader::new(main_mem, vm_create_config, vm.clone());
        loader.load().expect("Failed to load VM images");

        if let Err(e) = vm.init() {
            panic!("VM[{}] setup failed: {:?}", vm.id(), e);
        }
    }
}

fn config_guest_address(vm: &VM, main_memory: &VMMemoryRegion) {
    const MB: usize = 1024 * 1024;
    vm.with_config(|config| {
        if main_memory.is_identical() {
            debug!(
                "Adjusting kernel load address from {:#x} to {:#x}",
                config.image_config.kernel_load_gpa, main_memory.gpa
            );
            let mut kernel_addr = main_memory.gpa;
            if config.image_config.bios_load_gpa.is_some() {
                kernel_addr += MB * 2; // leave 2MB for BIOS
            }
            let dtb_addr = (main_memory.gpa + (main_memory.size().min(512 * MB) / 2).max(64 * MB))
                .align_up(2 * MB);

            config.image_config.kernel_load_gpa = kernel_addr;
            config.cpu_config.bsp_entry = kernel_addr;
            config.cpu_config.ap_entry = kernel_addr;
            config.image_config.dtb_load_gpa = Some(dtb_addr);
        }
    });
}

fn vm_alloc_memorys(vm_create_config: &AxVMCrateConfig, vm: &VM) {
    const MB: usize = 1024 * 1024;
    const ALIGN: usize = 2 * MB;

    for memory in &vm_create_config.kernel.memory_regions {
        match memory.map_type {
            VmMemMappingType::MapAlloc => {
                vm.alloc_memory_region(
                    Layout::from_size_align(memory.size, ALIGN).unwrap(),
                    Some(GuestPhysAddr::from(memory.gpa)),
                )
                .expect("Failed to allocate memory region for VM");
            }
            VmMemMappingType::MapIdentical => {
                vm.alloc_memory_region(Layout::from_size_align(memory.size, ALIGN).unwrap(), None)
                    .expect("Failed to allocate memory region for VM");
            }
        }
    }
}
