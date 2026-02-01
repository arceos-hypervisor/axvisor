use std::{fs, path::Path};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// An image entry in the image list file (one row in the registry TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    /// Unique image identifier (e.g. `qemu_x86_64_nimbos`, `evm3588_arceos`).
    pub name: String,
    /// Short human-readable description of the image.
    pub description: String,
    /// SHA-256 checksum of the image archive (hex string).
    pub sha256: String,
    /// Target architecture (e.g. `x86_64`, `aarch64`).
    pub arch: String,
    /// URL to download the image archive (e.g. `.tar.gz`).
    pub url: String,
}

/// An image list contains a list of [`ImageEntry`]s (top-level structure of the registry TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRegistry {
    /// All image entries from the registry.
    pub images: Vec<ImageEntry>,
}

impl ImageRegistry {
    pub fn load_from_file(path: &Path) -> Result<ImageRegistry> {
        let s = fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read image registry from {}: {e}", path.display()))?;
        toml::from_str(&s).map_err(|e| anyhow!("Invalid image list format: {e}"))
    }

    pub fn print(&self) {
        println!(
            "{:<25} {:<15} {:<50}",
            "Name", "Architecture", "Description"
        );
        println!("{}", "-".repeat(90));
        for image in &self.images {
            println!(
                "{:<25} {:<15} {:<50}",
                image.name, image.arch, image.description
            );
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ImageEntry> {
        self.images.iter()
    }

    pub fn find_by_name(&self, name: &str) -> Option<&ImageEntry> {
        self.iter().find(|e| e.name == name)
    }
}
