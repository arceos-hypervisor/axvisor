//! xtask/src/image.rs
//! Guest Image management commands for the Axvisor build configuration tool
//! (https://github.com/arceos-hypervisor/xtask).
//!
//! This module provides functionality to list, download, and remove
//! pre-built guest images for various supported boards and architectures. The images
//! are downloaded from a specified URL base and verified using SHA-256 checksums. The downloaded
//! images are automatically extracted to a specified output directory. Images can also be removed
//! from the temporary directory.
//! ! Usage examples:
//!! ```
//! // List available images
//! xtask image ls
//! // Download a specific image and automatically extract it (default behavior)
//! xtask image download evm3588_arceos --output-dir ./images
//! // Download a specific image without extracting
//! xtask image download evm3588_arceos --output-dir ./images --no-extract
//! // Remove a specific image from temp directory
//! xtask image rm evm3588_arceos
//! ```

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use tar::Archive;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Default directory for storing images and list files under the AxVisor
/// repository directory.
const DEFAULT_IMAGE_DIR: &str = ".images";

/// Name of the file containing the image list, retrieved from the remote
/// registry and stored locally.
const LIST_FILE_NAME: &str = "images.toml";

/// Default registry URL for the image list.
const DEFAULT_REGISTRY_URL: &str = "https://raw.githubusercontent.com/arceos-hypervisor/axvisor-guest-registry/refs/heads/main/default.toml";

/// Image management command line arguments.
#[derive(Parser)]
pub struct ImageArgs {
    #[command(subcommand)]
    pub command: ImageCommands,
}

/// Image management commands
#[derive(Subcommand)]
pub enum ImageCommands {
    /// List all available images
    Ls {
        /// Do not automatically sync image list from remote registry if the
        /// local image list is broken or missing.
        #[arg(long)]
        no_auto_sync: bool,
    },

    /// Download the specified image and automatically extract it
    Download {
        /// Name of the image to download
        image_name: String,

        /// Output directory for the downloaded image, defaults to
        /// "<axvisor-repo>/.images/"
        #[arg(short, long)]
        output_dir: Option<String>,

        /// Do not extract after download
        #[arg(long)]
        no_extract: bool,

        /// Do not automatically sync image list from remote registry even if
        /// the local image list is broken or missing
        #[arg(long)]
        no_auto_sync: bool,
    },

    /// Remove the specified image from temp directory
    Rm {
        /// Name of the image to remove
        image_name: String,
    },

    /// Synchronize image list from a remote registry
    Sync {
        /// Registry to sync from
        #[arg(short, long)]
        registry: Option<String>,
    },
}

/// An image entry in the image list file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageEntry {
    pub name: String,
    pub description: String,
    pub sha256: String,
    pub arch: String,
    pub url: String,
}

/// A image list contains a list of [`ImageEntry`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageList {
    pub images: Vec<ImageEntry>,
}

impl ImageList {
    /// Load image list from local image list file.
    fn load_local() -> Result<ImageList> {
        let path = get_image_list_file()?;
        let s = fs::read_to_string(&path).map_err(|e| {
            anyhow!(
                "Failed to read image list from {}: {e}. Run 'xtask image sync' to fetch the list.",
                path.display()
            )
        })?;
        toml::from_str(&s).map_err(|e| anyhow!("Invalid image list format: {e}"))
    }

    /// Load image list from local image list file, fetch from registry if the
    /// local image list file is missing or broken and `auto_sync` is set.
    pub async fn load(auto_sync: bool) -> Result<ImageList> {
        let result = Self::load_local();
        if result.is_ok() || !auto_sync {
            return result;
        }

        let err = result.unwrap_err();
        println!("Failed to load image list from local file: {err}. Auto syncing from registry...");
        sync_image_list(None)
            .await
            .map_err(|e| anyhow!("Auto sync failed: {e}"))?;
        Self::load_local()
    }

    /// Get all images from the local list file.
    pub async fn all(auto_sync: bool) -> Result<Vec<ImageEntry>> {
        Self::load(auto_sync).await.map(|list| list.images)
    }

    /// Find image by name in the local list file.
    pub async fn find_by_name(name: &str, auto_sync: bool) -> Result<Option<ImageEntry>> {
        Self::load(auto_sync)
            .await
            .map(|list| list.images.into_iter().find(|e| e.name == name))
    }
}

/// Get the path to the AxVisor repository directory.
fn get_axvisor_repo_dir() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR contains the path of the xtask crate, and we need to
    // get the parent directory to get the AxVisor repository directory.
    Ok(Path::new(&std::env::var("CARGO_MANIFEST_DIR")?)
        .parent()
        .ok_or_else(|| anyhow!("Failed to determine AxVisor repository directory"))?
        .to_path_buf())
}

/// Get the path to the default image directory under the AxVisor repository directory.
fn get_default_image_dir() -> Result<PathBuf> {
    Ok(get_axvisor_repo_dir()?.join(DEFAULT_IMAGE_DIR))
}

/// Get the path to the image list file stored locally.
fn get_image_list_file() -> Result<PathBuf> {
    Ok(get_default_image_dir()?.join(LIST_FILE_NAME))
}

/// Verify the SHA256 checksum of a file.
/// # Arguments
/// * `file_path` - The path to the file to verify
/// * `expected_sha256` - The expected SHA256 checksum as a hex string
/// # Returns
/// * `Result<bool>` - Result indicating whether the checksum matches
/// # Errors
/// * `anyhow::Error` - If any error occurs during the verification process
fn image_verify_sha256(file_path: &Path, expected_sha256: &str) -> Result<bool> {
    let mut file = fs::File::open(file_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let result = hasher.finalize();
    let actual_sha256 = format!("{result:x}");

    Ok(actual_sha256 == expected_sha256)
}

/// Download a URL to a local file path with optional progress output.
///
/// # Arguments
/// * `url` - URL to download
/// * `path` - Local path to write the file
/// * `progress_label` - If present, print progress lines with this label
async fn download_to_path(url: &str, path: &Path, progress_label: Option<&str>) -> Result<()> {
    let mut response = reqwest::get(url).await?;
    if !response.status().is_success() {
        return Err(anyhow!("Failed to download: HTTP {}", response.status()));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await?;
    let mut writer = BufWriter::new(file);

    let content_length = response.content_length();
    let mut downloaded = 0u64;

    while let Some(chunk) = response.chunk().await? {
        writer
            .write_all(&chunk)
            .await
            .map_err(|e| anyhow!("Error writing to file: {e}"))?;
        downloaded += chunk.len() as u64;
        if let Some(label) = progress_label {
            if let Some(total) = content_length {
                let percent = (downloaded * 100) / total;
                print!("\r{label}: {percent}% ({downloaded}/{total} bytes)");
            } else {
                print!("\r{label}: {downloaded} bytes");
            }
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }
    }

    writer
        .flush()
        .await
        .map_err(|e| anyhow!("Error flushing file: {e}"))?;
    Ok(())
}

/// List all available images
/// # Returns
/// * `Result<()>` - Result indicating success or failure
/// # Errors
/// * `anyhow::Error` - If any error occurs during the listing process
/// # Examples
/// ```
/// // List all available images
/// xtask image ls
/// ```
async fn image_list(auto_sync: bool) -> Result<()> {
    let images = ImageList::all(auto_sync).await?;

    // Print table headers with specific column widths
    println!(
        "{:<25} {:<15} {:<50}",
        "Name", "Architecture", "Description"
    );
    // Print a separator line for better readability
    println!("{}", "-".repeat(90));

    for image in &images {
        println!(
            "{:<25} {:<15} {:<50}",
            image.name, image.arch, image.description
        );
    }

    Ok(())
}

/// Download the specified image and optionally extract it
/// # Arguments
/// * `image_name` - The name of the image to download
/// * `output_dir` - Optional output directory to save the downloaded image
/// * `extract` - Whether to automatically extract the image after download (default: true)
/// # Returns
/// * `Result<()>` - Result indicating success or failure
/// # Errors
/// * `anyhow::Error` - If any error occurs during the download or extraction process
/// # Examples
/// ```
/// // Download the evm3588_arceos image to the ./images directory and automatically extract it
/// xtask image download evm3588_arceos --output-dir ./images
/// ```
async fn image_download(
    image_name: &str,
    output_dir: Option<String>,
    extract: bool,
    auto_sync: bool,
) -> Result<()> {
    let image = ImageList::find_by_name(image_name, auto_sync)
        .await?
        .ok_or_else(|| {
            anyhow!("Image not found: {image_name}. Use 'xtask image ls' to view available images")
        })?;

    let output_path = match output_dir {
        Some(dir) => {
            // Check if it's an absolute path
            let path = Path::new(&dir);
            if path.is_absolute() {
                // If it's an absolute path, use it directly
                path.join(format!("{image_name}.tar.gz"))
            } else {
                // If it's a relative path, base on current working directory
                let current_dir = std::env::current_dir()?;
                current_dir.join(path).join(format!("{image_name}.tar.gz"))
            }
        }
        None => get_default_image_dir()?.join(format!("{image_name}.tar.gz")),
    };

    // Check if file exists, if so verify SHA256
    if output_path.exists() {
        match image_verify_sha256(&output_path, &image.sha256) {
            Ok(true) => {
                println!("Image already exists and verified");
                return Ok(());
            }
            Ok(false) => {
                println!("Existing image verification failed, re-downloading");
                // Remove the invalid file before downloading
                let _ = fs::remove_file(&output_path);
            }
            Err(_) => {
                println!("Error verifying existing image, re-downloading");
                // Remove the potentially corrupted file before downloading
                let _ = fs::remove_file(&output_path);
            }
        }
    }

    println!("Downloading: {}", image.url);
    download_to_path(&image.url, &output_path, Some("Downloading")).await?;
    println!();

    match image_verify_sha256(&output_path, &image.sha256) {
        Ok(true) => {
            println!("Download completed and verified successfully");
        }
        Ok(false) => {
            // Remove the invalid downloaded file
            let _ = fs::remove_file(&output_path);
            return Err(anyhow!(
                "Download completed but file SHA256 verification failed"
            ));
        }
        Err(e) => {
            // Remove the potentially corrupted downloaded file
            let _ = fs::remove_file(&output_path);
            return Err(anyhow!(
                "Download completed but error verifying downloaded file: {e}"
            ));
        }
    }

    // If extract flag is true, extract the downloaded file
    if extract {
        println!("Extracting image...");

        // Determine extraction output directory
        let extract_dir = output_path
            .parent()
            .ok_or_else(|| anyhow!("Unable to determine parent directory of downloaded file"))?
            .join(&image.name);

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

/// Remove the specified image from temp directory
/// # Arguments
/// * `image_name` - The name of the image to remove
/// # Returns
/// * `Result<()>` - Result indicating success or failure
/// # Errors
/// * `anyhow::Error` - If any error occurs during the removal process
/// # Examples
/// ```
/// // Remove the evm3588_arceos image from temp directory
/// xtask image rm evm3588_arceos
/// ```
async fn image_remove(image_name: &str) -> Result<()> {
    let _image = ImageList::find_by_name(image_name, false)
        .await?
        .ok_or_else(|| {
            anyhow!("Image not found: {image_name}. Use 'xtask image ls' to view available images")
        })?;

    let default_dir = get_default_image_dir()?;
    let tar_file = default_dir.join(format!("{image_name}.tar.gz"));
    let extract_dir = default_dir.join(image_name);

    let mut removed = false;

    // Remove the tar file if it exists
    if tar_file.exists() {
        fs::remove_file(&tar_file)?;
        removed = true;
    }

    // Remove the extracted directory if it exists
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
        removed = true;
    }

    if !removed {
        println!("No files found for image: {image_name}");
    } else {
        println!("Image removed successfully");
    }

    Ok(())
}

/// Synchronize image list from a remote registry.
///
/// Downloads the list file from the given registry URL (or the default) and
/// saves it to the local image list path. Validates the downloaded content
/// as TOML before returning.
async fn sync_image_list(registry: Option<String>) -> Result<()> {
    let url = registry.unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string());
    let dir = get_default_image_dir()?;
    fs::create_dir_all(&dir)?;
    let list_path = get_image_list_file()?;

    println!("Syncing image list from: {}", url);
    download_to_path(&url, &list_path, Some("Syncing image list")).await?;
    println!();

    let s = fs::read_to_string(&list_path)?;
    toml::from_str::<ImageList>(&s).map_err(|e| {
        let _ = fs::remove_file(&list_path);
        anyhow!("Downloaded file is not a valid image list: {e}")
    })?;
    println!("Image list saved to {}", list_path.display());
    Ok(())
}

/// Main function to run image management commands
/// # Arguments
/// * `args` - The image command line arguments
/// # Returns
/// * `Result<()>` - Result indicating success or failure
/// # Errors
/// * `anyhow::Error` - If any error occurs during command execution
/// # Examples
/// ```
/// // Run image management commands
/// xtask image ls
/// xtask image download evm3588_arceos --output-dir ./images
/// xtask image rm evm3588_arceos
/// ```
pub async fn run_image(args: ImageArgs) -> Result<()> {
    match args.command {
        ImageCommands::Ls { no_auto_sync } => {
            image_list(!no_auto_sync).await?;
        }
        ImageCommands::Download {
            image_name,
            output_dir,
            no_extract,
            no_auto_sync,
        } => {
            image_download(&image_name, output_dir, !no_extract, !no_auto_sync).await?;
        }
        ImageCommands::Rm { image_name } => {
            image_remove(&image_name).await?;
        }
        ImageCommands::Sync { registry } => {
            sync_image_list(registry).await?;
        }
    }

    Ok(())
}
