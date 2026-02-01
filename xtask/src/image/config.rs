use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default registry URL for the image list.
pub const DEFAULT_REGISTRY_URL: &str = "https://raw.githubusercontent.com/arceos-hypervisor/axvisor-guest-registry/refs/heads/main/default.toml";

/// Path to the local image config file.
const IMAGE_CONFIG_PATH: &str = ".image.toml";

/// Default auto-sync threshold in seconds.
const DEFAULT_AUTO_SYNC_THRESHOLD: u64 = 60 * 60 * 24 * 7; // 7 days

/// Configuration for the image management.
///
/// This struct is used to parse image config file (in [`IMAGE_CONFIG_PATH`]).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ImageConfig {
    /// The path to the local storage of images.
    pub local_storage: PathBuf,
    /// The URL of the remote registry of images.
    pub registry: String,
    /// Automatically synchronize image list from remote registry if the local
    /// storage is broken, missing, or out of date.
    pub auto_sync: bool,
    /// The threshold in seconds to automatically synchronize image list from
    /// remote registry due to too long since last synchronization. 0 means
    /// never.
    pub auto_sync_threshold: u64,
}

impl ImageConfig {
    pub fn new_default() -> Self {
        Self {
            local_storage: std::env::temp_dir().join(".axvisor-images"),
            registry: DEFAULT_REGISTRY_URL.to_string(),
            auto_sync: true,
            auto_sync_threshold: DEFAULT_AUTO_SYNC_THRESHOLD,
        }
    }

    pub fn get_config_file_path(base_dir: &Path) -> Result<PathBuf> {
        Ok(base_dir.join(IMAGE_CONFIG_PATH))
    }

    pub fn read_config(base_dir: &Path) -> Result<Self> {
        let path = Self::get_config_file_path(base_dir)?;

        if !path.exists() {
            println!(
                "Creating default image config file at {}...",
                path.display()
            );
            Self::write_config(base_dir, &Self::new_default())
                .map_err(|e| anyhow!("Failed to create default image config file: {e}"))?;
            return Ok(Self::new_default());
        }

        let s = fs::read_to_string(&path)?;
        toml::from_str(&s).map_err(|e| anyhow!("Invalid image config file: {e}"))
    }

    pub fn write_config(base_dir: &Path, config: &Self) -> Result<()> {
        let path = Self::get_config_file_path(base_dir)?;
        fs::write(path, toml::to_string(config)?)
            .map_err(|e| anyhow!("Failed to write image config file: {e}"))
    }

    pub fn reset_config(base_dir: &Path) -> Result<()> {
        let default_config = Self::new_default();
        Self::write_config(base_dir, &default_config)
            .map_err(|e| anyhow!("Failed to reset image config file: {e}"))
    }
}

impl std::fmt::Display for ImageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Local storage: {}", self.local_storage.display())?;
        writeln!(f, "Registry: {}", self.registry)?;
        writeln!(f, "Auto sync: {}", self.auto_sync)?;

        if self.auto_sync_threshold == 0 {
            writeln!(f, "Auto sync threshold: 0 (never)")?;
        } else {
            let threshold_days = self.auto_sync_threshold / (60 * 60 * 24);
            writeln!(
                f,
                "Auto sync threshold: {} ({} day(s))",
                self.auto_sync_threshold, threshold_days
            )?;
        }

        Ok(())
    }
}
