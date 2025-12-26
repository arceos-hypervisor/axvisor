use axvm::GuestPhysAddr;

// use axvm::VMMemoryRegion;
use axvm::config::{AxVMCrateConfig, VMImageConfig, VMImagesConfig};
use axvmconfig::ImageLocation;

use crate::vmm::config::config::MemoryImage;

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

    pub fn load_images_fs(_config: &AxVMCrateConfig) -> anyhow::Result<VMImagesConfig> {
        todo!()
    }
}
