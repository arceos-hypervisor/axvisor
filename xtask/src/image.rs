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
//! // Download a specific image and automatically extract it
//! xtask image download evm3588_arceos --output-dir ./images
//! // Download a specific image without extracting
//! xtask image download evm3588_arceos --output-dir ./images --no-extract
//! // Remove a specific image from temp directory
//! xtask image rm evm3588_arceos
//! ```

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use sha2::{Sha256, Digest};
use std::path::{Path};
use std::process::Command;
use std::fs;
use std::env;
use std::io::Read;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Base URL for downloading images
const IMAGE_URL_BASE: &str = "https://github.com/arceos-hypervisor/axvisor-guest/releases/download/v0.0.18/";

/// Image management command line arguments.
#[derive(Parser)]
pub struct ImageArgs {
    #[command(subcommand)]
    pub command: ImageCommands,
}

/// Image management commands
#[derive(Subcommand)]
pub enum ImageCommands {
    /// List all available image
    Ls,
    /// Download the specified image and automatically extract it
    #[command(alias = "pull")]
    Download {
        image_name: String,
        #[arg(short, long)]
        output_dir: Option<String>,
        #[arg(short, long, help = "Automatically extract after download (default: true)")]
        extract: Option<bool>,
    },
    /// Remove the specified image from temp directory
    Rm {
        image_name: String,
    },
}

/// Representation of a guest image
#[derive(Debug, Clone, Copy)]
struct Image {
    pub name: &'static str,
    pub description: &'static str,
    pub sha256: &'static str,
    pub arch: &'static str,
}

/// Supported guest images
impl Image {
    pub const EVM3588_ARCEOS: Self = Self {
        name: "evm3588_arceos",
        description: "ArceOS for EVM3588 development board",
        sha256: "5a7a967e8d45a1dab0ae709e38e4e6855667f54cdafff323cbef02ba83bacb19",
        arch: "aarch64",
    };
    
    pub const EVM3588_LINUX: Self = Self {
        name: "evm3588_linux",
        description: "Linux for EVM3588 development board",
        sha256: "bce9f15f6afc5d442b06525d7a353c821ded36c3414c29d957700625116982c1",
        arch: "aarch64",
    };
    
    pub const ORANGEPI_ARCEOS: Self = Self {
        name: "orangepi_arceos",
        description: "ArceOS for Orange Pi development board",
        sha256: "85089cbe778d42dc6acd216768562297d00ae4ceb1fe89713851008726ca0bf1",
        arch: "aarch64",
    };
    
    pub const ORANGEPI_LINUX: Self = Self {
        name: "orangepi_linux",
        description: "Linux for Orange Pi development board",
        sha256: "c0f2d69b860d9d3fd9fc20c1c0b5bccdb183cf06c4f4c65ba7fceaff6e31920c",
        arch: "aarch64",
    };
    
    pub const PHYTIUMPI_ARCEOS: Self = Self {
        name: "phytiumpi_arceos",
        description: "ArceOS for Phytium Pi development board",
        sha256: "94f1b78498391b4dd9ddf4b56553dfd0e83deec7c6e8fb30812784a0115c5de7",
        arch: "aarch64",
    };
    
    pub const PHYTIUMPI_LINUX: Self = Self {
        name: "phytiumpi_linux",
        description: "Linux for Phytium Pi development board",
        sha256: "e66d8caa00e0c2c1b4a793810eb8a081856eba1c7d5f2826bf7ee8dbe7a34524",
        arch: "aarch64",
    };
    
    pub const QEMU_ARCEOS_AARCH64: Self = Self {
        name: "qemu_arceos_aarch64",
        description: "ArceOS for QEMU aarch64 virtualization",
        sha256: "cc46b6049d71593c5a5264e63a883a4e689a52af316212d751afe442034279c6",
        arch: "aarch64",
    };
    
    pub const QEMU_ARCEOS_RISCV64: Self = Self {
        name: "qemu_arceos_riscv64",
        description: "ArceOS for QEMU riscv64 virtualization",
        sha256: "0907eccce7624e499395dd1fa1ff5526ee43d6009ed5947cde427d9cc6d726e0",
        arch: "riscv64",
    };
    
    pub const QEMU_ARCEOS_X86_64: Self = Self {
        name: "qemu_arceos_x86_64",
        description: "ArceOS for QEMU x86_64 virtualization",
        sha256: "ebb401331de9d4cf9de6bf8d7d0d0a26fcd25e2ffdb6c0b670999922efc26ebe",
        arch: "x86_64",
    };
    
    pub const QEMU_LINUX_AARCH64: Self = Self {
        name: "qemu_linux_aarch64",
        description: "Linux for QEMU aarch64 virtualization",
        sha256: "6ef339d4122b8c5a0bb10a73c03506c6484131a8cd30d63ef73c4c1da402ef85",
        arch: "aarch64",
    };
    
    pub const QEMU_LINUX_RISCV64: Self = Self {
        name: "qemu_linux_riscv64",
        description: "Linux for QEMU riscv64 virtualization",
        sha256: "589fa1034fe133ab64418d54b7b70ffd818991ed943df346f49a584adfe9c001",
        arch: "riscv64",
    };
    
    pub const QEMU_LINUX_X86_64: Self = Self {
        name: "qemu_linux_x86_64",
        description: "Linux for QEMU x86_64 virtualization",
        sha256: "57e9221c0e61a326dee9f8950ec36a55c2bf9f5b3581bbd1282d143a36da2fe1",
        arch: "x86_64",
    };
    
    pub const ROC_RK3568_PC_ARCEOS: Self = Self {
        name: "roc-rk3568-pc_arceos",
        description: "ArceOS for ROC-RK3568-PC development board",
        sha256: "a68d4981a0053278b7f90c11ede1661c037310223dd3188ffe4a4e272a7e3cdd",
        arch: "aarch64",
    };
    
    pub const ROC_RK3568_PC_LINUX: Self = Self {
        name: "roc-rk3568-pc_linux",
        description: "Linux for ROC-RK3568-PC development board",
        sha256: "53a8db12bd8b5b75e1f29847cec6486c8d9e3bf58a03ca162322662ff61eb7fa",
        arch: "aarch64",
    };
    
    pub const TAC_E400_PLC_ARCEOS: Self = Self {
        name: "tac-e400-plc_arceos",
        description: "ArceOS for TAC-E400-PLC industrial control board",
        sha256: "1ff39e83e4af2aaaae57ff6fc853f0e51efeb63463b2f4f1d425d9105d8a62f8",
        arch: "aarch64",
    };
    
    pub const TAC_E400_PLC_LINUX: Self = Self {
        name: "tac-e400-plc_linux",
        description: "Linux for TAC-E400-PLC industrial control board",
        sha256: "28bf16ccee10e0dae911f8c787d24a625e0e0d4071f1964e6d97c56f68b7a4ab",
        arch: "aarch64",
    };
    
    /// Get all supported images
    pub fn all() -> &'static [Image] {
        &[
            Self::EVM3588_ARCEOS,
            Self::EVM3588_LINUX,
            Self::ORANGEPI_ARCEOS,
            Self::ORANGEPI_LINUX,
            Self::PHYTIUMPI_ARCEOS,
            Self::PHYTIUMPI_LINUX,
            Self::QEMU_ARCEOS_AARCH64,
            Self::QEMU_ARCEOS_RISCV64,
            Self::QEMU_ARCEOS_X86_64,
            Self::QEMU_LINUX_AARCH64,
            Self::QEMU_LINUX_RISCV64,
            Self::QEMU_LINUX_X86_64,
            Self::ROC_RK3568_PC_ARCEOS,
            Self::ROC_RK3568_PC_LINUX,
            Self::TAC_E400_PLC_ARCEOS,
            Self::TAC_E400_PLC_LINUX,
        ]
    }
    
    /// Find image by name
    pub fn find_by_name(name: &str) -> Option<&'static Image> {
        Self::all().iter().find(|image| image.name == name)
    }
}

/// Verify the SHA256 checksum of a file
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
fn image_list() -> Result<()> {
    // Retrieve all images from the database or storage
    let images = Image::all();
    
    // Print table headers with specific column widths
    println!("{:<25} {:<30} {:<50}", "Name", "Architecture", "Description");
    // Print a separator line for better readability
    println!("{}", "-".repeat(90));
    
    // Iterate through each image and print its details
    for image in images {
        // Print image information formatted to match column widths
        println!("{:<25} {:<15} {:<50}",
                 // Image name
                 image.name,
                 // Architecture type
                 image.arch,
                 image.description);
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
/// // Or use the pull alias
/// xtask image pull evm3588_arceos --output-dir ./images
/// ```
async fn image_download(image_name: &str, output_dir: Option<String>, extract: bool) -> Result<()> {
    let image = Image::find_by_name(image_name)
        .ok_or_else(|| anyhow!("Image not found: {image_name}. Use 'xtask image ls' to view available images"))?;
    
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
        None => {
            // If not specified, use system temporary directory
            let temp_dir = env::temp_dir();
            temp_dir.join("axvisor").join(format!("{image_name}.tar.gz"))
        }
    };
    
    // Check if file exists, if so verify SHA256
    if output_path.exists() {
        match image_verify_sha256(&output_path, image.sha256) {
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
    
    // Ensure target directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Build download URL
    let download_url = format!("{}{}.tar.gz", IMAGE_URL_BASE, image.name);
    println!("Downloading: {download_url}");
    
    // Use reqwest to download the file
    let mut response = reqwest::get(&download_url).await?;
    if !response.status().is_success() {
        return Err(anyhow!("Failed to download file: HTTP {}", response.status()));
    }
    
    // Create file with buffered writer for efficient streaming
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&output_path)
        .await?;
    let mut writer = BufWriter::new(file);
    
    // Get content length for progress reporting (if available)
    let content_length = response.content_length();
    let mut downloaded = 0u64;
    
    // Stream the response body to file using chunks
    while let Some(chunk) = response.chunk().await? {
        // Write chunk to file
        writer.write_all(&chunk).await
            .map_err(|e| anyhow!("Error writing to file: {}", e))?;
        
        // Update progress
        downloaded += chunk.len() as u64;
        if let Some(total) = content_length {
            let percent = (downloaded * 100) / total;
            print!("\rDownloading: {}% ({}/{} bytes)", percent, downloaded, total);
        } else {
            print!("\rDownloaded: {} bytes", downloaded);
        }
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
    }
    
    // Flush the writer to ensure all data is written to disk
    writer.flush().await
        .map_err(|e| anyhow!("Error flushing file: {}", e))?;
    
    println!("\nDownload completed");
    
    
    // Verify downloaded file
    match image_verify_sha256(&output_path, image.sha256) {
        Ok(true) => {
            println!("Download completed and verified successfully");
        }
        Ok(false) => {
            // Remove the invalid downloaded file
            let _ = fs::remove_file(&output_path);
            return Err(anyhow!("Downloaded file SHA256 verification failed"));
        }
        Err(e) => {
            // Remove the potentially corrupted downloaded file
            let _ = fs::remove_file(&output_path);
            return Err(anyhow!("Error verifying downloaded file: {e}"));
        }
    }
    
    // If extract flag is true, extract the downloaded file
    if extract {
        println!("Extracting image...");
        
        // Determine extraction output directory
        let extract_dir = output_path.parent()
            .ok_or_else(|| anyhow!("Unable to determine parent directory of downloaded file"))?
            .join(image_name);
        
        // Ensure extraction directory exists
        fs::create_dir_all(&extract_dir)?;
        
        // Use tar command to extract file
        let mut child = Command::new("tar")
            .arg("-xzf")
            .arg(&output_path)
            .arg("-C")
            .arg(&extract_dir)
            .spawn()?;
        
        let status = child.wait()?;
        if !status.success() {
            return Err(anyhow!("Extraction failed, tar exit code: {status}"));
        }
        
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
fn image_remove(image_name: &str) -> Result<()> {
    // Check if the image name is valid by looking it up
    let _image = Image::find_by_name(image_name)
        .ok_or_else(|| anyhow!("Image not found: {image_name}. Use 'xtask image ls' to view available images"))?;
    
    let temp_dir = env::temp_dir().join("axvisor");
    let tar_file = temp_dir.join(format!("{image_name}.tar.gz"));
    let extract_dir = temp_dir.join(image_name);
    
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
/// // Or use the pull alias
/// xtask image pull evm3588_arceos --output-dir ./images
/// xtask image rm evm3588_arceos
/// ```
pub async fn run_image(args: ImageArgs) -> Result<()> {
    match args.command {
        ImageCommands::Ls => {
            image_list()?;
        }
        ImageCommands::Download { image_name, output_dir, extract } => {
            image_download(&image_name, output_dir, extract.unwrap_or(true)).await?;
        }
        ImageCommands::Rm { image_name } => {
            image_remove(&image_name)?;
        }
    }
    
    Ok(())
}
