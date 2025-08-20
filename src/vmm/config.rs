use alloc::string::ToString;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use std::os::arceos::{api::task::AxCpuMask, modules::axconfig, modules::axhal::hvconfig};

use axaddrspace::MappingFlags;

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
    let instance_cpus = axconfig::SMP - reserved_cpus;

    // Set reserved CPUs.
    RESERVED_CPUS.store(reserved_cpus, Ordering::Release);

    // Set instance CPUs.
    INSTANCE_CPUS.store(instance_cpus, Ordering::Release);

    info!(
        "Creating host VM...\n{} CPUS reserved\n{} CPUS available for instances",
        reserved_cpus, instance_cpus
    );

    // Set CPU bitmap for host VM.
    // Note: The first reserved_cpus CPUs are reserved for the host VM currently.
    // The rest CPUs are available for instances.
    // We need to ensure that the reserved CPUs' id starts from 0.
    let mut instance_cpu_cnt = 0;
    for i in 0..axconfig::SMP {
        if !hvconfig::core_id_is_reserved(i) {
            unsafe {
                INSTANCE_CPU_MASK = INSTANCE_CPU_MASK | (1 << i);
            }
            instance_cpu_cnt += 1;
        }
    }

    if instance_cpu_cnt != instance_cpus {
        error!(
            "CPU mask does not match instance CPU count: {} != {}",
            instance_cpu_cnt, instance_cpus
        );
    }

    let instance_cpu_mask = AxCpuMask::from_raw_bits(unsafe { INSTANCE_CPU_MASK });

    if instance_cpu_mask.len() != instance_cpus {
        let cpu_ids: Vec<usize> = instance_cpu_mask.into_iter().collect();
        error!(
            "CPU mask length does not match instance CPU count: {} != {}, known instance cpus: {:?}",
            instance_cpu_mask.len(),
            instance_cpus,
            cpu_ids,
        );
    }

    let mut host_vm_cfg = AxVMConfig::new_host(0, "host".to_string(), reserved_cpus);

    // Map host VM memory regions.
    for region in axhal::host_memory_regions() {
        let mapping_flags = MappingFlags::from(region.flags);

        host_vm_cfg.append_memory_region(VmMemConfig {
            gpa: region.paddr.as_usize(),
            size: region.size,
            // region.flags is of type `MemRegionFlags`.
            // MappingFlags is required here.
            flags: mapping_flags.bits(),
            map_type: VmMemMappingType::MapIentical,
        });
    }
    // I DO NOT know why Linux in x14sbi want to access this
    // host_vm_cfg.append_memory_region(VmMemConfig {
    //     gpa: 0xe0000,
    //     size: 0x10000,
    //     flags: (MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE).bits(),
    //     map_type: VmMemMappingType::MapIentical,
    // });

    // I DO NOT know why Linux in x14sbi want to access this
    // [ 17.808062 11:205 axvisor::vmm::vcpus:361] Unhandled VM-Exit
    // NestedPageFault {
    //     addr: GPA:0xf624e808,
    //     access_flags: READ,
    // }
    host_vm_cfg.append_memory_region(VmMemConfig {
        gpa: 0xf624e000,
        size: 0x1000,
        flags: (MappingFlags::READ | MappingFlags::WRITE | MappingFlags::DEVICE).bits(),
        map_type: VmMemMappingType::MapIentical,
    });

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
