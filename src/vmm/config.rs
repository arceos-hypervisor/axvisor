use axaddrspace::GuestPhysAddr;
use axerrno::AxResult;
use axvm::config::{AxVMConfig, AxVMCrateConfig, VmMemMappingType};
use core::alloc::Layout;

use crate::vmm::{VM, images::ImageLoader, vm_list::push_vm};

#[cfg(target_arch = "aarch64")]
use crate::vmm::fdt::*;

use alloc::sync::Arc;

#[allow(clippy::module_inception, dead_code)]
pub mod config {
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Default static VM configs. Used when no VM config is provided.
    pub fn default_static_vm_configs() -> Vec<&'static str> {
        vec![]
    }

    /// Read VM configs from filesystem
    #[cfg(feature = "fs")]
    pub fn filesystem_vm_configs() -> Vec<String> {
        use axstd::fs;

        // Try to read config files from a predefined directory
        let config_dir = "configs/vms";
        let mut configs = Vec::new();

        if let Ok(entries) = fs::read_dir(config_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // Check if the file has a .toml extension
                let path_str = path.as_str();
                if path_str.ends_with(".toml")
                    && let Ok(content) = fs::read_to_string(path_str)
                {
                    configs.push(content);
                }
            }
        }

        configs
    }

    /// Fallback function for when "fs" feature is not enabled
    #[cfg(not(feature = "fs"))]
    pub fn filesystem_vm_configs() -> Vec<String> {
        Vec::new()
    }

    include!(concat!(env!("OUT_DIR"), "/vm_configs.rs"));
}

pub fn get_vm_dtb_arc(_vm_cfg: &AxVMConfig) -> Option<Arc<[u8]>> {
    #[cfg(target_arch = "aarch64")]
    {
        let cache_lock = dtb_cache().lock();
        if let Some(dtb) = cache_lock.get(&_vm_cfg.id()) {
            return Some(Arc::from(dtb.as_slice()));
        }
    }
    None
}

pub fn init_guest_vms() {
    // Initialize the DTB cache in the fdt module
    #[cfg(target_arch = "aarch64")]
    {
        init_dtb_cache();
    }

    // First try to get configs from filesystem if fs feature is enabled
    let mut gvm_raw_configs = config::filesystem_vm_configs();

    // If no filesystem configs found, fallback to static configs
    if gvm_raw_configs.is_empty() {
        let static_configs = config::static_vm_configs();
        // Convert static configs to String type
        gvm_raw_configs.extend(static_configs.into_iter().map(|s| s.into()));
    }

    for raw_cfg_str in gvm_raw_configs {
        if let Err(e) = init_guest_vm(&raw_cfg_str) {
            error!("Failed to initialize guest VM: {:?}", e);
        }
    }
}

pub fn init_guest_vm(raw_cfg: &str) -> AxResult {
    let vm_create_config =
        AxVMCrateConfig::from_toml(raw_cfg).expect("Failed to resolve VM config");

    if let Some(linux) = super::images::get_image_header(&vm_create_config) {
        debug!(
            "VM[{}] Linux header: {:#x?}",
            vm_create_config.base.id, linux
        );
    }

    #[cfg(target_arch = "aarch64")]
    let mut vm_config = AxVMConfig::from(vm_create_config.clone());

    #[cfg(not(target_arch = "aarch64"))]
    let vm_config = AxVMConfig::from(vm_create_config.clone());

    // Handle FDT-related operations for aarch64
    #[cfg(target_arch = "aarch64")]
    handle_fdt_operations(&mut vm_config, &vm_create_config);

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

    // Load corresponding images for VM.
    info!("VM[{}] created success, loading images...", vm.id());

    let mut loader = ImageLoader::new(main_mem, vm_create_config, vm.clone());
    loader.load().expect("Failed to load VM images");

    if let Err(e) = vm.init() {
        panic!("VM[{}] setup failed: {:?}", vm.id(), e);
    }

    Ok(())
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
            VmMemMappingType::MapReserved => {
                info!("VM[{}] map same region: {:#x?}", vm.id(), memory);
                let layout = Layout::from_size_align(memory.size, ALIGN).unwrap();
                vm.map_reserved_memory_region(layout, Some(GuestPhysAddr::from(memory.gpa)))
                    .expect("Failed to map memory region for VM");
            }
        }
    }
}
