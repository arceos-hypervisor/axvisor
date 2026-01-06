use axvm::GuestPhysAddr;

// use axvm::VMMemoryRegion;
use axvm::config::{AxVMCrateConfig, VMImageConfig, VMImagesConfig};
use axvmconfig::ImageLocation;

use crate::config::config::MemoryImage;

mod linux;

pub fn load_images(config: &AxVMCrateConfig) -> anyhow::Result<VMImagesConfig> {
    match config.kernel.image_location {
        None | Some(ImageLocation::Fs) => {
            #[cfg(feature = "fs")]
            {
                fs::load_images_fs(config)
            }
            #[cfg(not(feature = "fs"))]
            {
                Err(anyhow::anyhow!(
                    "Filesystem feature is not enabled, cannot load images from fs"
                ))
            }
        }
        Some(ImageLocation::Memory) => load_images_mem(config),
    }
}

fn load_images_mem(config: &AxVMCrateConfig) -> anyhow::Result<VMImagesConfig> {
    let memory_image = memory_image(config);

    // Load kernel image
    let kernel = VMImageConfig {
        gpa: config.kernel.kernel_load_addr.map(GuestPhysAddr::from),
        data: memory_image.kernel.to_vec(),
    };

    let bios = memory_image.bios.map(|bios| VMImageConfig {
        gpa: config.kernel.bios_load_addr.map(GuestPhysAddr::from),
        data: bios.to_vec(),
    });

    let dtb = memory_image.dtb.map(|dtb| VMImageConfig {
        gpa: config.kernel.dtb_load_addr.map(GuestPhysAddr::from),
        data: dtb.to_vec(),
    });

    let ramdisk = memory_image.ramdisk.map(|ramdisk| VMImageConfig {
        gpa: config.kernel.ramdisk_load_addr.map(GuestPhysAddr::from),
        data: ramdisk.to_vec(),
    });

    Ok(VMImagesConfig {
        kernel,
        bios,
        dtb,
        ramdisk,
    })
}

fn memory_image(config: &AxVMCrateConfig) -> &'static MemoryImage {
    let images = super::config::config::get_memory_images();
    for img in images.iter() {
        if img.id == config.base.id {
            return img;
        }
    }
    panic!("Cannot find memory image for VM id {}", config.base.id);
}

#[cfg(feature = "fs")]
pub mod fs {
    use super::*;
    use alloc::vec::Vec;

    pub fn load_images_fs(config: &AxVMCrateConfig) -> anyhow::Result<VMImagesConfig> {
        // Load kernel image
        let kernel = load_image_file(&config.kernel.kernel_path)?;
        let kernel = VMImageConfig {
            gpa: config.kernel.kernel_load_addr.map(GuestPhysAddr::from),
            data: kernel,
        };

        // Load BIOS image if configured
        let bios = if let Some(bios_path) = &config.kernel.bios_path {
            let bios_data = load_image_file(bios_path)?;
            Some(VMImageConfig {
                gpa: config.kernel.bios_load_addr.map(GuestPhysAddr::from),
                data: bios_data,
            })
        } else {
            None
        };

        // Load DTB image if configured
        let dtb = if let Some(dtb_path) = &config.kernel.dtb_path {
            let dtb_data = load_image_file(dtb_path)?;
            Some(VMImageConfig {
                gpa: config.kernel.dtb_load_addr.map(GuestPhysAddr::from),
                data: dtb_data,
            })
        } else {
            None
        };

        // Load ramdisk image if configured
        let ramdisk = if let Some(ramdisk_path) = &config.kernel.ramdisk_path {
            let ramdisk_data = load_image_file(ramdisk_path)?;
            Some(VMImageConfig {
                gpa: config.kernel.ramdisk_load_addr.map(GuestPhysAddr::from),
                data: ramdisk_data,
            })
        } else {
            None
        };

        Ok(VMImagesConfig {
            kernel,
            bios,
            dtb,
            ramdisk,
        })
    }

    fn load_image_file(path: &str) -> anyhow::Result<Vec<u8>> {
        use axstd::io::Read;

        let mut file = axstd::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open image file '{}': {}", path, e))?;

        let metadata = file
            .metadata()
            .map_err(|e| anyhow::anyhow!("Failed to get metadata for '{}': {}", path, e))?;

        let file_size = metadata.len() as usize;
        let mut buffer = Vec::with_capacity(file_size);

        file.read_to_end(&mut buffer)
            .map_err(|e| anyhow::anyhow!("Failed to read image file '{}': {}", path, e))?;

        Ok(buffer)
    }
}
