//! Image registry data structures and parsing.
//!
//! Defines `ImageEntry` and `ImageRegistry` for the TOML-based image list format.

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
    /// Loads the image registry from a TOML file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the registry TOML file (e.g. `images.toml`)
    ///
    /// # Returns
    ///
    /// * `Ok(ImageRegistry)` - Parsed registry
    /// * `Err` - File read or TOML parse error
    pub fn load_from_file(path: &Path) -> Result<ImageRegistry> {
        let s = fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read image registry from {}: {e}", path.display()))?;
        toml::from_str(&s).map_err(|e| anyhow!("Invalid image list format: {e}"))
    }

    /// Prints the image list in a formatted table to stdout (name, architecture, description).
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

    /// Returns an iterator over all image entries.
    pub fn iter(&self) -> impl Iterator<Item = &ImageEntry> {
        self.images.iter()
    }

    /// Looks up an image by name.
    ///
    /// # Arguments
    ///
    /// * `name` - Image name to search for (e.g. `evm3588_arceos`)
    ///
    /// # Returns
    ///
    /// * `Some(&ImageEntry)` - Matching image entry if found
    /// * `None` - No image with the given name
    pub fn find_by_name(&self, name: &str) -> Option<&ImageEntry> {
        self.iter().find(|e| e.name == name)
    }
}
