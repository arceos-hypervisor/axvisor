//! Local image storage management.
//!
//! Provides `Storage` for managing a local image directory and its registry index.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, path::PathBuf};

use anyhow::{Result, anyhow};

use super::config::ImageConfig;
use super::download::{download_to_path, image_verify_sha256};
use super::registry::ImageRegistry;

/// Filename of the image registry index inside the local storage directory.
pub const REGISTRY_FILENAME: &str = "images.toml";

/// Filename storing the last sync timestamp (Unix seconds) inside the local storage directory.
const LAST_SYNC_FILENAME: &str = ".last_sync";

/// Local image storage backed by a directory and an image registry index.
pub struct Storage {
    /// Root path of the local image storage directory.
    pub path: PathBuf,
    /// Parsed image registry (list of available images).
    pub image_registry: ImageRegistry,
}

impl Storage {
    /// Creates a storage instance from an existing local directory.
    ///
    /// Loads the image registry from `images.toml` in the storage path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the local storage directory (must contain `images.toml`)
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - Storage loaded successfully
    /// * `Err` - Directory or registry file read/parse error
    pub fn new(path: PathBuf) -> Result<Self> {
        let registry_filepath = Self::registry_filepath(&path);
        let image_registry = ImageRegistry::load_from_file(&registry_filepath)?;
        Ok(Self {
            path,
            image_registry,
        })
    }

    /// Creates storage, falling back to syncing from the remote registry if local load fails.
    /// When local load succeeds and `auto_sync_threshold` is non-zero, checks the last sync
    /// time (stored in `.last_sync` under the storage path) and syncs from the remote registry
    /// if the threshold in seconds has been exceeded (or no last sync time exists).
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the local storage directory
    /// * `registry` - URL of the remote registry to sync from when local storage is invalid or stale
    /// * `auto_sync_threshold` - Seconds since last sync before auto-updating; 0 means never update when load succeeds
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - Storage from local dir or from synced registry
    /// * `Err` - Both local load and sync failed
    pub async fn new_with_auto_sync(
        path: PathBuf,
        registry: String,
        auto_sync_threshold: u64,
    ) -> Result<Self> {
        let storage = match Self::new(path.clone()) {
            Ok(storage) => storage,
            Err(e) => {
                println!("Error while loading local storage: {e}");
                println!("Auto syncing from registry {registry}...");
                let storage = Self::new_from_registry(registry, path).await?;
                return Ok(storage);
            }
        };

        if auto_sync_threshold == 0 {
            return Ok(storage);
        }

        let now = Self::current_unix_timestamp()?;
        let last_sync = Self::read_last_sync_time(&storage.path);
        let need_sync = match last_sync {
            None => true,
            Some(ts) => now.saturating_sub(ts) >= auto_sync_threshold,
        };

        if !need_sync {
            return Ok(storage);
        }

        println!(
            "Last sync was {} (threshold: {}s). Auto syncing from registry {registry}...",
            last_sync
                .map(|ts| format!("{}s ago", now - ts))
                .unwrap_or_else(|| "never".to_string()),
            auto_sync_threshold
        );

        // backup registry file so we can restore on sync failure.
        let registry_path = Self::registry_filepath(&storage.path);
        let registry_backup = fs::read_to_string(&registry_path)
            .map_err(|e| anyhow!("Failed to read registry file: {e}"))?;

        match Self::new_from_registry(registry, path).await {
            Ok(new_storage) => Ok(new_storage),
            Err(e) => {
                println!("Auto sync failed: {e}");
                println!("Restoring previous registry and using existing storage.");

                fs::write(&registry_path, registry_backup)
                    .map_err(|e| anyhow!("Failed to write registry file: {e}"))?;

                Ok(storage)
            }
        }
    }

    /// Creates storage by downloading the registry index from the remote URL.
    ///
    /// Creates the storage directory if it does not exist.
    ///
    /// # Arguments
    ///
    /// * `registry` - URL of the registry TOML file to download
    /// * `path` - Path to the local storage directory
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - Registry downloaded and storage created
    /// * `Err` - Download, directory creation, or parse error
    pub async fn new_from_registry(registry: String, path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&path).map_err(|e| anyhow!("Failed to create directory: {e}"))?;

        let registry_filepath = Self::registry_filepath(&path);

        download_to_path(&registry, &registry_filepath, Some("Syncing image list")).await?;
        Self::write_last_sync_time(&path)?;

        let image_registry = ImageRegistry::load_from_file(&registry_filepath)?;
        println!("Image list saved to {}", registry_filepath.display());

        Ok(Self {
            path,
            image_registry,
        })
    }

    /// Creates storage from config, optionally auto-syncing when local storage is invalid.
    ///
    /// # Arguments
    ///
    /// * `config` - Image config (storage path, registry URL, auto-sync settings)
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - Storage loaded or synced according to config
    /// * `Err` - Load or sync failed
    pub async fn new_from_config(config: &ImageConfig) -> Result<Self> {
        if config.auto_sync {
            Self::new_with_auto_sync(
                config.local_storage.clone(),
                config.registry.clone(),
                config.auto_sync_threshold,
            )
            .await
        } else {
            Self::new(config.local_storage.clone())
        }
    }
}

impl Storage {
    /// Returns the path to the registry index file within the storage directory.
    pub fn registry_filepath(storage_path: &Path) -> PathBuf {
        storage_path.join(REGISTRY_FILENAME)
    }

    /// Returns the current Unix timestamp (seconds since epoch).
    fn current_unix_timestamp() -> Result<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow!("System time error: {e}"))
            .map(|d| d.as_secs())
    }

    /// Returns the path to the file storing the last sync timestamp.
    fn last_sync_filepath(storage_path: &Path) -> PathBuf {
        storage_path.join(LAST_SYNC_FILENAME)
    }

    /// Reads the last sync time (Unix seconds) from storage.
    /// Returns `None` if the file is missing or invalid; never returns an error.
    /// Prints a short message when the file exists but cannot be read or parsed.
    fn read_last_sync_time(storage_path: &Path) -> Option<u64> {
        let path = Self::last_sync_filepath(storage_path);
        if !path.exists() {
            return None;
        }
        let s = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                println!(
                    "Note: could not read last sync file {}: {e}; treating as no previous sync.",
                    path.display()
                );
                return None;
            }
        };
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        match s.parse::<u64>() {
            Ok(ts) => Some(ts),
            Err(_) => {
                println!(
                    "Note: last sync file {} has invalid content; treating as no previous sync.",
                    path.display()
                );
                None
            }
        }
    }

    /// Writes the current time as the last sync timestamp (Unix seconds).
    fn write_last_sync_time(storage_path: &Path) -> Result<()> {
        let now = Self::current_unix_timestamp()?;
        let path = Self::last_sync_filepath(storage_path);
        fs::write(&path, now.to_string())
            .map_err(|e| anyhow!("Failed to write last sync file: {e}"))
    }

    /// Returns the path where an image archive (`.tar.gz`) would be stored.
    pub fn image_path(storage_path: &Path, image_name: &str) -> PathBuf {
        storage_path.join(format!("{image_name}.tar.gz"))
    }
}

impl Storage {
    /// Downloads an image to a specific path and verifies its SHA256 checksum.
    ///
    /// Skips download if the file already exists and matches the expected checksum.
    /// Re-downloads on checksum mismatch.
    ///
    /// # Arguments
    ///
    /// * `image_name` - Name of the image in the registry
    /// * `output_path` - Destination path for the `.tar.gz` file (must not be a directory)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Image downloaded and verified successfully
    /// * `Err` - Image not found, download failed, or checksum verification failed
    pub async fn download_image_to(&self, image_name: &str, output_path: &Path) -> Result<()> {
        // find image in registry
        let image = self
            .image_registry
            .find_by_name(image_name)
            .ok_or_else(|| {
                anyhow!(
                    "Image not found: {image_name}. Use 'xtask image ls' to view available images"
                )
            })?;

        // check if output path is a directory
        if output_path.is_dir() {
            return Err(anyhow!(
                "Output path is a directory: {}",
                output_path.display()
            ));
        }

        // check if output path exists
        if output_path.exists() {
            match image_verify_sha256(&output_path, &image.sha256) {
                Ok(true) => {
                    println!("Image already exists and verified");
                    return Ok(());
                }
                Ok(false) => {
                    println!("Existing image verification failed");
                }
                Err(e) => {
                    println!("Error verifying existing image: {e}");
                }
            }

            println!("Removing existing image for re-downloading...");
            let _ = fs::remove_file(&output_path);
        }

        // download image
        println!("Downloading: {}", image.url);

        download_to_path(&image.url, &output_path, Some("Downloading")).await?;

        // verify SHA256 checksum
        let err = match image_verify_sha256(&output_path, &image.sha256) {
            Ok(true) => {
                println!("Download completed and verified successfully");
                return Ok(());
            }
            Ok(false) => {
                anyhow!("Image downloaded but verification failed: SHA256 verification failed")
            }
            Err(e) => {
                anyhow!("Image downloaded but verification failed: Error verifying image: {e}")
            }
        };

        println!("{err}");
        let _ = fs::remove_file(&output_path);

        Err(err)
    }

    /// Downloads an image to the default location in local storage.
    ///
    /// Equivalent to `download_image_to(image_name, path/"{image_name}.tar.gz")`.
    ///
    /// # Arguments
    ///
    /// * `image_name` - Name of the image in the registry
    ///
    /// # Returns
    ///
    /// * `Ok(PathBuf)` - Path to the downloaded image file
    /// * `Err` - Same as [`download_image_to`](Self::download_image_to)
    pub async fn download_image(&self, image_name: &str) -> Result<PathBuf> {
        let output_path = Self::image_path(&self.path, image_name);
        self.download_image_to(image_name, &output_path).await?;
        Ok(output_path)
    }

    /// Removes an image from local storage (archive and extracted directory).
    ///
    /// # Arguments
    ///
    /// * `image_name` - Name of the image to remove
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - At least one file or directory was removed
    /// * `Ok(false)` - No matching files found
    /// * `Err` - File/directory removal error
    pub async fn remove_image(&self, image_name: &str) -> Result<bool> {
        let mut anything_removed = false;
        let output_path = Self::image_path(&self.path, image_name);
        if output_path.exists() {
            fs::remove_file(&output_path)?;
            anything_removed = true;
        }
        let extract_dir = self.path.join(image_name);
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir)?;
            anything_removed = true;
        }
        Ok(anything_removed)
    }
}
