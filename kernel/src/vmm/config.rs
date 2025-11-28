use std::string::ToString;

use alloc::vec::Vec;
use axvm::{
    AxVMConfig, CpuId,
    config::{AxVMCrateConfig, CpuNumType, MemoryKind},
};

pub fn get_guest_prelude_vmconfig() -> anyhow::Result<Vec<AxVMCrateConfig>> {
    let mut vm_configs = Vec::new();
    // First try to get configs from filesystem if fs feature is enabled
    let mut gvm_raw_configs = config::filesystem_vm_configs();

    // If no filesystem configs found, fallback to static configs
    if gvm_raw_configs.is_empty() {
        let static_configs = config::static_vm_configs();
        if static_configs.is_empty() {
            info!("Static VM configs are empty.");
            info!("Now axvisor will entry the shell...");
        } else {
            info!("Using static VM configs.");
        }

        gvm_raw_configs.extend(static_configs.into_iter().map(|s| s.to_string()));
    }
    for raw in gvm_raw_configs {
        let vm_config: AxVMCrateConfig = toml::from_str(&raw)?;
        vm_configs.push(vm_config);
    }

    Ok(vm_configs)
}

pub fn build_vmconfig(cfg: AxVMCrateConfig) -> anyhow::Result<AxVMConfig> {
    let mut cpu_num = CpuNumType::Alloc(1);
    if let Some(num) = cfg.base.cpu_num {
        cpu_num = CpuNumType::Alloc(num);
    }
    if let Some(ref ids) = cfg.base.cpu_ids {
        cpu_num = CpuNumType::Fixed(ids.iter().map(|&id| CpuId::new(id)).collect());
    }

    let image_config = super::images::load_images(&cfg)?;

    let mut memory_regions = vec![];

    for region in &cfg.kernel.memory_regions {
        let mem_region = match region.map_type {
            axvmconfig::VmMemMappingType::MapAlloc => MemoryKind::Vmem {
                gpa: region.gpa.into(),
                size: region.size,
            },
            axvmconfig::VmMemMappingType::MapIdentical => {
                MemoryKind::Identical { size: region.size }
            }
            axvmconfig::VmMemMappingType::MapReserved => MemoryKind::Reserved {
                hpa: region.gpa.into(),
                size: region.size,
            },
        };

        memory_regions.push(mem_region);
    }

    Ok(AxVMConfig {
        id: cfg.base.id,
        name: cfg.base.name,
        cpu_num,
        image_config,
        memory_regions,
        interrupt_mode: cfg.devices.interrupt_mode,
    })
}

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
        use axstd::io::{BufReader, Read};

        let config_dir = "/guest/vm_default";

        let mut configs = Vec::new();

        debug!("Read VM config files from filesystem.");

        let entries = match fs::read_dir(config_dir) {
            Ok(entries) => {
                info!("Find dir: {config_dir}");
                entries
            }
            Err(_e) => {
                info!("NOT find dir: {config_dir} in filesystem");
                return configs;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            // Check if the file has a .toml extension
            let path_str = path.as_str();
            debug!("Considering file: {path_str}");
            if path_str.ends_with(".toml") {
                let toml_file = fs::File::open(path_str).expect("Failed to open file");
                let file_size = toml_file
                    .metadata()
                    .expect("Failed to get file metadata")
                    .len() as usize;

                info!("File {path_str} size: {file_size}");

                if file_size == 0 {
                    warn!("File {path_str} is empty");
                    continue;
                }

                let mut file = BufReader::new(toml_file);
                let mut buffer = vec![0u8; file_size];
                match file.read_exact(&mut buffer) {
                    Ok(()) => {
                        debug!(
                            "Successfully read config file {} as bytes, size: {}",
                            path_str,
                            buffer.len()
                        );
                        // Convert to string
                        let content = alloc::string::String::from_utf8(buffer)
                            .expect("Failed to convert bytes to UTF-8 string");

                        if content.contains("[base]")
                            && content.contains("[kernel]")
                            && content.contains("[devices]")
                        {
                            configs.push(content);
                            info!(
                                "TOML config: {path_str} is valid, start the virtual machine directly now. "
                            );
                        } else {
                            warn!(
                                "File {path_str} does not appear to contain valid VM config structure"
                            );
                        }
                    }
                    Err(e) => {
                        error!("Failed to read file {path_str}: {e:?}");
                    }
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
