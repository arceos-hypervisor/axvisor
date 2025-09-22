use core::alloc::Layout;
use axaddrspace::GuestPhysAddr;
use axvm::{
    VMMemoryRegion,
    config::{AxVMConfig, AxVMCrateConfig, VmMemMappingType},
};
use fdt_parser::Fdt;
use memory_addr::MemoryAddr;

use crate::vmm::{
    fdt::*, images::ImageLoader, vm_list::push_vm, VM
};

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use lazyinit::LazyInit;
use spin::Mutex;

pub static GENERATED_DTB_CACHE: LazyInit<Mutex<BTreeMap<usize, Arc<[u8]>>>> = LazyInit::new();

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

pub fn get_developer_provided_dtb(vm_cfg: &AxVMConfig, crate_config: &AxVMCrateConfig) -> Option<Vec<u8>> {
    match crate_config.kernel.image_location.as_deref() {
        Some("memory") => {
            let vm_imags = config::get_memory_images()
                .iter()
                .find(|&v| v.id == vm_cfg.id())?;

            if let Some(dtb) = vm_imags.dtb {
                return Some(dtb.to_vec());
            }
        },
        #[cfg(feature = "fs")]
        Some("fs") => {
            use std::io::{BufReader, Read};
            use axerrno::ax_err_type;
            if let Some(dtb_path) = &crate_config.kernel.dtb_path {
                let (dtb_file, dtb_size) = crate::vmm::images::open_image_file(&dtb_path).unwrap();
                info!("DTB file in fs, size: {:x}", dtb_size);

                let mut file = BufReader::new(dtb_file);
                let mut dtb_buffer = vec![0; dtb_size];

                file.read_exact(&mut dtb_buffer).map_err(|err| {
                    ax_err_type!(
                        Io,
                        format!("Failed in reading from file {}, err {:?}", dtb_path, err)
                    )
                }).unwrap();
                return Some(dtb_buffer);
            }
        },
        _ => unimplemented!(
            "Check your \"image_location\" in config.toml, \"memory\" and \"fs\" are supported,\n."
        ),
    }
    None
}

pub fn get_vm_dtb_arc(vm_cfg: &AxVMConfig) -> Option<Arc<[u8]>> {
    if let Some(cache) = GENERATED_DTB_CACHE.get() {
        let cache_lock = cache.lock();
        if let Some(dtb) = cache_lock.get(&vm_cfg.id()) {
            return Some(dtb.clone());
        }
    }
    None
}

pub fn init_guest_vms() {
    GENERATED_DTB_CACHE.init_once(Mutex::new(BTreeMap::new()));

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

        let host_fdt_bytes = get_host_fdt();
        let host_fdt = Fdt::from_bytes(host_fdt_bytes)
            .map_err(|e| format!("Failed to parse FDT: {:#?}", e))
            .expect("Failed to parse FDT");
        set_phys_cpu_sets(&mut vm_config, &host_fdt, &vm_create_config);

        if let Some(provided_dtb) = get_developer_provided_dtb(&vm_config, &vm_create_config) {
            info!("VM[{}] found DTB , parsing...", vm_config.id());
            update_provided_fdt(&provided_dtb, host_fdt_bytes, &vm_create_config);
        } else {
            info!(
                "VM[{}] DTB not found, generating based on the configuration file.",
                vm_config.id()
            );
            setup_guest_fdt_from_vmm(host_fdt_bytes, &mut vm_config, &vm_create_config);
        }

        // Overlay VM config with the given DTB.
        if let Some(dtb_arc) = get_vm_dtb_arc(&vm_config) {
            let dtb = dtb_arc.as_ref();
            parse_passthrough_devices_address(&mut vm_config, dtb);
            parse_vm_interrupt(&mut vm_config, dtb);
        } else {
            error!(
                "VM[{}] DTB not found in memory, skipping...",
                vm_config.id()
            );
        }

        // info!("after parse_vm_interrupt, crate VM[{}] with config: {:#?}", vm_config.id(), vm_config);
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
