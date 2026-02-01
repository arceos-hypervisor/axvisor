//! Guest Image management commands for the Axvisor build configuration tool
//!
//! This module provides functionality to list, download, and remove
//! pre-built guest images for various supported boards and architectures. The images
//! are downloaded from a specified URL base and verified using SHA-256 checksums. The downloaded
//! images are automatically extracted to a specified output directory. Images can also be removed
//! from the temporary directory.
//!
//! # Usage examples
//!
//! ```
//! // List available images
//! xtask image ls
//! // Download a specific image and automatically extract it (default behavior)
//! xtask image download evm3588_arceos --output-dir ./images
//! // Download a specific image without extracting
//! xtask image download evm3588_arceos --output-dir ./images --no-extract
//! // Remove a specific image from temp directory
//! xtask image rm evm3588_arceos
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use flate2::read::GzDecoder;
use tar::Archive;

mod config;
mod download;
mod registry;
mod storage;

use config::ImageConfig;
use storage::Storage;

/// Image management command line arguments.
#[derive(Parser)]
pub struct ImageArgs {
    #[command(flatten)]
    pub overrides: ImageConfigOverrides,

    /// Image subcommand to run: `ls`, `download`, `rm`, or `sync`.
    #[command(subcommand)]
    pub command: ImageCommands,
}

#[derive(Parser)]
pub struct ImageConfigOverrides {
    /// The path to the local storage of images. Override the config file.
    #[arg(short('S'), long, global = true)]
    pub local_storage: Option<PathBuf>,

    /// The URL of the remote registry of images. Override the config file.
    #[arg(short('R'), long, global = true)]
    pub registry: Option<String>,

    /// Do not sync from remote registry even if the local image storage is
    /// broken, missing, or out of date. Override the config file.
    #[arg(short('N'), long, global = true)]
    pub no_auto_sync: bool,

    /// The threshold in seconds to automatically synchronize image list from
    /// remote registry. 0 means never. Override the config file.
    #[arg(long, global = true)]
    pub auto_sync_threshold: Option<u64>,
}

impl ImageConfigOverrides {
    pub fn apply_on(&self, config: &mut ImageConfig) {
        if let Some(local_storage) = self.local_storage.as_ref() {
            config.local_storage = local_storage.clone();
        }
        if let Some(registry) = self.registry.as_ref() {
            config.registry = registry.clone();
        }
        if self.no_auto_sync {
            config.auto_sync = false;
        }
        if let Some(auto_sync_threshold) = self.auto_sync_threshold {
            config.auto_sync_threshold = auto_sync_threshold;
        }
    }
}

/// Image management commands
#[derive(Subcommand)]
pub enum ImageCommands {
    /// List all available images.
    Ls,

    /// Download the specified image and automatically extract it.
    Download {
        /// Name of the image to download.
        image_name: String,

        /// Output directory for the downloaded image, defaults to
        /// "/tmp/.axvisor-images/".
        #[arg(short, long)]
        output_dir: Option<String>,

        /// Do not extract after download.
        #[arg(long)]
        no_extract: bool,
    },

    /// Remove the specified image from temp directory.
    Rm {
        /// Name of the image to remove.
        image_name: String,
    },

    /// Synchronize image list from a remote registry.
    Sync,

    /// Reset the image config file to default.
    Defconfig,
}

/// Returns the path to the AxVisor repository root (parent of the xtask crate).
///
/// # Returns
///
/// * `Ok(PathBuf)` - Path to the repository root
/// * `Err` - If `CARGO_MANIFEST_DIR` is unset or the parent path cannot be determined
fn get_axvisor_repo_dir() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR contains the path of the xtask crate, and we need to
    // get the parent directory to get the AxVisor repository directory.
    Ok(Path::new(&std::env::var("CARGO_MANIFEST_DIR")?)
        .parent()
        .ok_or_else(|| anyhow!("Failed to determine AxVisor repository directory"))?
        .to_path_buf())
}

impl ImageArgs {
    /// Loads image configuration, merging CLI overrides with values from the config file.
    ///
    /// CLI arguments override config file values when both are specified.
    ///
    /// # Returns
    ///
    /// * `Ok(ImageConfig)` - Merged configuration
    /// * `Err` - If config file read fails or AxVisor repo path cannot be determined
    pub async fn get_config(&self) -> Result<ImageConfig> {
        let mut config = ImageConfig::read_config(&get_axvisor_repo_dir()?)?;
        self.overrides.apply_on(&mut config);
        Ok(config)
    }

    /// Executes the selected image subcommand (`ls`, `download`, `rm`, `sync`, or `defconfig`).
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Subcommand completed successfully
    /// * `Err` - Subcommand failed (e.g. config load, download, or sync error)
    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            ImageCommands::Ls => {
                self.list_images().await?;
            }
            ImageCommands::Download {
                image_name,
                output_dir,
                no_extract,
            } => {
                self.download_image(image_name, output_dir.as_deref(), !no_extract)
                    .await?;
            }
            ImageCommands::Rm { image_name } => {
                self.remove_image(image_name).await?;
            }
            ImageCommands::Sync => {
                self.sync_registry().await?;
            }
            ImageCommands::Defconfig => {
                ImageConfig::reset_config(&get_axvisor_repo_dir()?)?;
            }
        }

        Ok(())
    }

    /// Lists all available images from the local registry to stdout.
    ///
    /// Uses merged config and triggers auto-sync if enabled and local storage is out of date.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Images listed successfully
    /// * `Err` - Config load, storage init, or sync failed
    pub async fn list_images(&self) -> Result<()> {
        let config = self.get_config().await?;
        let storage = Storage::new_from_config(&config).await?;

        storage.image_registry.print();

        Ok(())
    }

    /// Downloads the specified image and optionally extracts it.
    ///
    /// # Arguments
    ///
    /// * `image_name` - Name of the image to download (e.g. `evm3588_arceos`)
    /// * `output_dir` - If `Some`, write the `.tar.gz` to this directory; if `None`, use
    ///   local storage path from config
    /// * `extract` - If `true`, extract the archive after download
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Image downloaded (and extracted if requested) successfully
    /// * `Err` - Config load, storage init, download, or extraction failed
    pub async fn download_image(
        &self,
        image_name: &str,
        output_dir: Option<&str>,
        extract: bool,
    ) -> Result<()> {
        let config = self.get_config().await?;
        let storage = Storage::new_from_config(&config).await?;

        let output_path = match output_dir {
            Some(dir) => {
                let path = Path::new(&dir);
                let output_dir = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir()?.join(path)
                };
                let output_path = output_dir.join(format!("{image_name}.tar.gz"));
                storage.download_image_to(image_name, &output_path).await?;
                output_path
            }
            None => {
                storage.download_image(image_name).await?;
                Storage::image_path(&storage.path, image_name)
            }
        };

        if extract {
            println!("Extracting image...");

            // Determine extraction output directory
            let extract_dir = output_path
                .parent()
                .ok_or_else(|| anyhow!("Unable to determine parent directory of downloaded file"))?
                .join(image_name);

            // Ensure extraction directory exists
            fs::create_dir_all(&extract_dir)?;

            // Open the compressed tar file
            let tar_gz = fs::File::open(&output_path)?;
            let decoder = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(decoder);

            // Extract the archive
            archive.unpack(&extract_dir)?;

            println!("Image extracted to: {}", extract_dir.display());
        }
        Ok(())
    }

    /// Removes the specified image from local storage (both `.tar.gz` and extracted directory).
    ///
    /// # Arguments
    ///
    /// * `image_name` - Name of the image to remove
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Removal completed; prints a message if no files were found
    /// * `Err` - Config load, storage init, or file removal failed
    pub async fn remove_image(&self, image_name: &str) -> Result<()> {
        let config = self.get_config().await?;
        let storage = Storage::new_from_config(&config).await?;

        let removed = storage.remove_image(image_name).await?;
        if !removed {
            println!("No files found for image: {image_name}");
        } else {
            println!("Image removed successfully");
        }
        Ok(())
    }

    /// Synchronizes the image list from the remote registry to local storage.
    ///
    /// Overwrites the local `images.toml` with the registry contents.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Sync completed successfully
    /// * `Err` - Config load, registry fetch, or file write failed
    pub async fn sync_registry(&self) -> Result<()> {
        let config: ImageConfig = self.get_config().await?;
        let _ = Storage::new_from_registry(config.registry, config.local_storage).await?;
        Ok(())
    }
}

/// Dispatches and runs the image subcommand (ls, download, rm, sync) from parsed CLI arguments.
///
/// # Arguments
///
/// * `args` - Parsed image CLI arguments (subcommand and its options)
///
/// # Returns
///
/// * `Ok(())` - Subcommand completed successfully
/// * `Err` - Subcommand failed (e.g. list load, download, checksum, sync, or remove error)
///
/// # Examples
///
/// ```ignore
/// xtask image ls
/// xtask image download evm3588_arceos --output-dir ./images
/// xtask image rm evm3588_arceos
/// ```
pub async fn run_image(args: ImageArgs) -> Result<()> {
    args.execute().await
}
