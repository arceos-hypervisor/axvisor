use alloc::string::ToString;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::arceos::{api::task::AxCpuMask, modules::axconfig};

use axvm::config::{AxVMConfig, AxVMCrateConfig};

use crate::vmm::{VM, images::load_vm_images, vm_list::push_vm};

#[allow(clippy::module_inception)]
pub mod config {
    use alloc::vec::Vec;

    /// Default static VM configs. Used when no VM config is provided.
    #[allow(dead_code)]
    pub fn default_static_vm_configs() -> Vec<&'static str> {
        vec![
            // #[cfg(target_arch = "x86_64")]
            // core::include_str!("../../configs/vms/nimbos-x86_64.toml"),
            // #[cfg(target_arch = "aarch64")]
            // core::include_str!("../../configs/vms/nimbos-aarch64.toml"),
            // #[cfg(target_arch = "riscv64")]
            // core::include_str!("../../configs/vms/nimbos-riscv64.toml"),
        ]
    }

    include!(concat!(env!("OUT_DIR"), "/vm_configs.rs"));
}

pub fn init_guest_vms() {
    let gvm_raw_configs = config::static_vm_configs();

    for raw_cfg_str in gvm_raw_configs {
        let vm_create_config =
            AxVMCrateConfig::from_toml(raw_cfg_str).expect("Failed to resolve VM config");
        let vm_config = AxVMConfig::from(vm_create_config.clone());

        info!("Creating VM [{}] {:?}", vm_config.id(), vm_config.name());

        // Create VM.
        let vm = VM::new(vm_config).expect("Failed to create VM");
        push_vm(vm.clone());

        // Load corresponding images for VM.
        info!("VM[{}] created success, loading images...", vm.id());
        load_vm_images(vm_create_config, vm.clone()).expect("Failed to load VM images");
    }
}

static mut INSTANCE_CPU_MASK: usize = 0;
// Cores reserved for host VM.
static RESERVED_CPUS: AtomicUsize = AtomicUsize::new(0);
// Cores reserved for instances.
static INSTANCE_CPUS: AtomicUsize = AtomicUsize::new(0);

pub fn init_host_vm() {
    use std::os::arceos::modules::axhal;

    use axvm::config::AxVMConfig;
    use axvmconfig::{VmMemConfig, VmMemMappingType};

    let reserved_cpus = axhal::hvheader::HvHeader::get().reserved_cpus() as usize;

    // Set reserved CPUs.
    RESERVED_CPUS.store(reserved_cpus, Ordering::Release);

    // Set instance CPUs.
    INSTANCE_CPUS.store(axconfig::SMP - reserved_cpus, Ordering::Release);

    info!(
        "Creating host VM...\n{} CPUS reserved\n{} CPUS available for instances",
        reserved_cpus,
        axconfig::SMP - reserved_cpus
    );

    // Set CPU bitmap for host VM.
    // Note: The first reserved_cpus CPUs are reserved for the host VM currently.
    // The rest CPUs are available for instances.
    // We need to ensure that the reserved CPUs' id starts from 0.
    for i in 0..axconfig::SMP {
        if i >= reserved_cpus {
            unsafe {
                INSTANCE_CPU_MASK = INSTANCE_CPU_MASK | (1 << i);
            }
        } else {
            break;
        }
    }

    let mut host_vm_cfg = AxVMConfig::new_host(0, "host".to_string(), reserved_cpus);

    // Map host VM memory regions.
    for region in axhal::host_memory_regions() {
        host_vm_cfg.append_memory_region(VmMemConfig {
            gpa: region.paddr.as_usize(),
            size: region.size,
            flags: region.flags.bits(),
            map_type: VmMemMappingType::MapIentical,
        });
    }

    // Create VM.
    let vm =
        VM::new_host(host_vm_cfg, axhal::get_linux_context_list()).expect("Failed to create VM");
    push_vm(vm.clone());
}

pub fn get_reserved_cpus() -> usize {
    RESERVED_CPUS.load(Ordering::Acquire)
}

pub fn get_instance_cpus() -> usize {
    INSTANCE_CPUS.load(Ordering::Acquire)
}

pub fn descrease_instance_cpus() {
    if get_instance_cpus() == 0 {
        warn!("No instance CPUs");
    }

    INSTANCE_CPUS.fetch_sub(1, Ordering::Release);
}

pub fn get_instance_cpus_mask() -> AxCpuMask {
    AxCpuMask::from_raw_bits(unsafe { INSTANCE_CPU_MASK })
}
