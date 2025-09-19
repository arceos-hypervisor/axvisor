use core::alloc::Layout;

use alloc::string::ToString;

use axaddrspace::GuestPhysAddr;
use axvm::{
    VMMemoryRegion,
    config::{AxVMConfig, AxVMCrateConfig, PassThroughDeviceConfig, VmMemMappingType},
};
use memory_addr::MemoryAddr;

use crate::vmm::{
    VM, fdt::{parse_passthrough_devices_address, parse_vm_interrupt}, images::ImageLoader, vm_list::push_vm,
};

// 添加用于存储生成的DTB的全局静态变量
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use lazyinit::LazyInit;
use spin::Mutex;

// 用于存储生成的DTB数据的全局缓存
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

pub fn get_vm_dtb(vm_cfg: &AxVMConfig) -> Option<&'static [u8]> {
    let vm_imags = config::get_memory_images()
        .iter()
        .find(|&v| v.id == vm_cfg.id())?;

    if let Some(dtb) = vm_imags.dtb {
        return Some(dtb);
    }

    None
}

/// 获取VM的DTB数据（返回Arc<[u8]>，支持缓存数据）
pub fn get_vm_dtb_arc(vm_cfg: &AxVMConfig) -> Option<Arc<[u8]>> {
    // 首先尝试返回静态DTB数据
    let vm_imags = config::get_memory_images()
        .iter()
        .find(|&v| v.id == vm_cfg.id())?;

    if let Some(dtb) = vm_imags.dtb {
        // 将&'static [u8]转换为Arc<[u8]>
        return Some(Arc::from(dtb));
    }

    // 如果内存镜像中没有DTB，则尝试从生成的DTB缓存中获取
    if let Some(cache) = GENERATED_DTB_CACHE.get() {
        let cache_lock = cache.lock();
        if let Some(dtb) = cache_lock.get(&vm_cfg.id()) {
            // 返回缓存中的Arc引用
            return Some(dtb.clone());
        }
    }

    None
}

pub fn init_guest_vms() {
    // 初始化DTB缓存
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

        // info!("vm_create_config: {:#?}", vm_create_config);
        // info!("before parse_vm_interrupt, crate VM[{}] with config: {:#?}", vm_config.id(), vm_config);

        // let bootarg: usize = unsafe { std::os::arceos::modules::axhal::get_bootarg() };
        // if bootarg != 0 {
        //     crate::vmm::fdt::parse_fdt(bootarg, &mut vm_config);
        // }
        if let Some(dtb) = get_vm_dtb(&vm_config) {
            info!("VM[{}] found DTB , parsing...", vm_config.id());

            // crate::vmm::fdt::parse_vm_fdt(&mut vm_config, dtb);
            //test
            let bootarg = dtb.as_ptr() as usize;
            crate::vmm::fdt::parse_fdt(bootarg, &mut vm_config, &vm_create_config);
            //test
        } else {
            info!(
                "VM[{}] DTB not found, generating based on the configuration file.",
                vm_config.id()
            );

            let bootarg: usize = std::os::arceos::modules::axhal::get_bootarg();
            if bootarg != 0 {
                crate::vmm::fdt::parse_fdt(bootarg, &mut vm_config, &vm_create_config);
            }
        }

        // Overlay VM config with the given DTB.
        if let Some(dtb_arc) = get_vm_dtb_arc(&vm_config) {
            let dtb = dtb_arc.as_ref();
            parse_passthrough_devices_address(&mut vm_config, dtb);
            parse_vm_interrupt(&mut vm_config, dtb);
        } else {
            warn!(
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
