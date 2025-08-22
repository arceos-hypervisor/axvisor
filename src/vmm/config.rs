use alloc::string::ToString;
use alloc::vec::Vec;

use axaddrspace::MappingFlags;
use axvm::config::{
    AxVMConfig, AxVMCrateConfig, PassThroughDeviceConfig, VmMemConfig, VmMemMappingType,
};

use crate::vmm::{VM, images::load_vm_images, vm_list::push_vm};

#[allow(clippy::module_inception)]
pub mod config {
    use alloc::vec::Vec;

    /// Default static VM configs. Used when no VM config is provided.
    #[allow(dead_code)]
    pub fn default_static_vm_configs() -> Vec<&'static str> {
        vec![
            #[cfg(target_arch = "x86_64")]
            core::include_str!("../../configs/vms/nimbos-x86_64.toml"),
            #[cfg(target_arch = "aarch64")]
            core::include_str!("../../configs/vms/nimbos-aarch64.toml"),
            #[cfg(target_arch = "riscv64")]
            core::include_str!("../../configs/vms/nimbos-riscv64.toml"),
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

    let mut dram_regions = Vec::new();
    for mem in fdt.memory() {
        for region in mem.regions() {
            if region.size == 0 {
                continue;
            }
            dram_regions.push((region.address as usize, region.size));
        }
    }

    for mem in fdt.memory() {
        for region in mem.regions() {
            // Skip empty regions
            if region.size == 0 {
                continue;
            }
            warn!("DTB memory region: {:?}", region);
            vm_cfg.add_memory_region(VmMemConfig {
                gpa: region.address as usize,
                size: region.size,
                flags: (MappingFlags::READ
                    | MappingFlags::WRITE
                    | MappingFlags::EXECUTE
                    | MappingFlags::USER)
                    .bits(),
                map_type: VmMemMappingType::MapIdentical,
            });
        }
    }

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
                    let end = start + size;
                    if vm_cfg.contains_memory_range(&(start..end)) {
                        trace!(
                            "Skipping DTB node {} with register address {:#x} and size {:#x} as it overlaps with existing memory regions",
                            node.name(),
                            reg.address,
                            size
                        );
                        continue;
                    }

                    let pt_dev = PassThroughDeviceConfig {
                        name: node.name().to_string(),
                        base_gpa: reg.address as _,
                        base_hpa: reg.address as _,
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

        // Load corresponding images for VM.
        info!("VM[{}] created success, loading images...", vm.id());
        load_vm_images(vm_create_config, vm.clone()).expect("Failed to load VM images");
    }
}
