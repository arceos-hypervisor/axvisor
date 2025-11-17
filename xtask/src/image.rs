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
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

/// Base URL for downloading images
const IMAGE_URL_BASE: &str = "https://github.com/arceos-hypervisor/axvisor-guest/releases/download/v0.0.17/";

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
        sha256: "fb0bb9011d4997e30debdf1d6c3769d7061ae9db1c087382952401ebabe47190",
        arch: "aarch64",
    };
    
    pub const EVM3588_LINUX: Self = Self {
        name: "evm3588_linux",
        description: "Linux for EVM3588 development board",
        sha256: "1affd34099d648fdb29104c93e190c24769c58a4c5ee31ac4ba2a593969dfb4c",
        arch: "aarch64",
    };
    
    pub const ORANGEPI_ARCEOS: Self = Self {
        name: "orangepi_arceos",
        description: "ArceOS for Orange Pi development board",
        sha256: "f84f711f3ad735b365de292ac341ea517ec5f7068c5629a4144749e741737e1d",
        arch: "aarch64",
    };
    
    pub const ORANGEPI_LINUX: Self = Self {
        name: "orangepi_linux",
        description: "Linux for Orange Pi development board",
        sha256: "3e6193e1a4f78fc9f6f07f920b4678a11e0f176227ca46cb4c9cdc8e2b2a9922",
        arch: "aarch64",
    };
    
    pub const PHYTIUMPI_ARCEOS: Self = Self {
        name: "phytiumpi_arceos",
        description: "ArceOS for Phytium Pi development board",
        sha256: "1ad199483198fd1bccc2c0fc8bad1a00d6c14ee46a4021898e40a49654a8e26f",
        arch: "aarch64",
    };
    
    pub const PHYTIUMPI_LINUX: Self = Self {
        name: "phytiumpi_linux",
        description: "Linux for Phytium Pi development board",
        sha256: "c546a575e30a604f83dea8cba660434cc5cabec4cc3ee7db3c943821e391a078",
        arch: "aarch64",
    };
    
    pub const QEMU_ARCEOS_AARCH64: Self = Self {
        name: "qemu_arceos_aarch64",
        description: "ArceOS for QEMU aarch64 virtualization",
        sha256: "fcad4aff7906cd6c14d41889d72cd7ce82ea78bf8c21bca19a4f3db0ac627c5b",
        arch: "aarch64",
    };
    
    pub const QEMU_ARCEOS_RISCV64: Self = Self {
        name: "qemu_arceos_riscv64",
        description: "ArceOS for QEMU riscv64 virtualization",
        sha256: "7f758fdfa32e1bf7e2f79b288af72500598f3fe767f4743370b6897f59e159a0",
        arch: "riscv64",
    };
    
    pub const QEMU_ARCEOS_X86_64: Self = Self {
        name: "qemu_arceos_x86_64",
        description: "ArceOS for QEMU x86_64 virtualization",
        sha256: "c15ed57f5969b8744fec89d32cc5f5a5704f1c3ac8ea40b4df44ae33d57c2dfa",
        arch: "x86_64",
    };
    
    pub const QEMU_LINUX_AARCH64: Self = Self {
        name: "qemu_linux_aarch64",
        description: "Linux for QEMU aarch64 virtualization",
        sha256: "608ce5e37a5417d056cc48f7aceb4dd2179ef86cbf59ec9dd21d207f43216bb8",
        arch: "aarch64",
    };
    
    pub const QEMU_LINUX_RISCV64: Self = Self {
        name: "qemu_linux_riscv64",
        description: "Linux for QEMU riscv64 virtualization",
        sha256: "546676ca7d30ae63762b30a41d77dde2558b823009838fac7bae2680bc5975c0",
        arch: "riscv64",
    };
    
    pub const QEMU_LINUX_X86_64: Self = Self {
        name: "qemu_linux_x86_64",
        description: "Linux for QEMU x86_64 virtualization",
        sha256: "8ceb1835814bee915fcc1a105a7403e7e883eeeb838d1abd7dac834218a48118",
        arch: "x86_64",
    };
    
    pub const ROC_RK3568_PC_ARCEOS: Self = Self {
        name: "roc-rk3568-pc_arceos",
        description: "ArceOS for ROC-RK3568-PC development board",
        sha256: "bb1c4314c933b2eb0425c78e5b719bdfe77e9f3d002183711270b84bbb08b716",
        arch: "aarch64",
    };
    
    pub const ROC_RK3568_PC_LINUX: Self = Self {
        name: "roc-rk3568-pc_linux",
        description: "Linux for ROC-RK3568-PC development board",
        sha256: "a353a900213eaef08a1851d689bca16f26c338d7660489c26a0324b1b2922571",
        arch: "aarch64",
    };
    
    pub const TAC_E400_PLC_ARCEOS: Self = Self {
        name: "tac-e400-plc_arceos",
        description: "ArceOS for TAC-E400-PLC industrial control board",
        sha256: "cbdb1ae9fd58ee9741a7dee235f94a8809f81a06f165bea89f86e394921be6ae",
        arch: "aarch64",
    };
    
    pub const TAC_E400_PLC_LINUX: Self = Self {
        name: "tac-e400-plc_linux",
        description: "Linux for TAC-E400-PLC industrial control board",
        sha256: "1459de9f11f9f6f2c97b241a1deccc02238d296577cba1b6e71004954adeaa19",
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
    let actual_sha256 = format!("{:x}", result);
    
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
    // Return Ok to indicate successful execution
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
        .ok_or_else(|| anyhow!("Image not found: {}. Use 'xtask image ls' to view available images", image_name))?;
    
    let output_path = match output_dir {
        Some(dir) => {
            // Check if it's an absolute path
            let path = Path::new(&dir);
            if path.is_absolute() {
                // If it's an absolute path, use it directly
                path.join(format!("{}.tar.gz", image_name))
            } else {
                // If it's a relative path, base on current working directory
                let current_dir = std::env::current_dir()?;
                current_dir.join(path).join(format!("{}.tar.gz", image_name))
            }
        }
        None => {
            // If not specified, use system temporary directory
            let temp_dir = env::temp_dir();
            temp_dir.join("axvisor").join(format!("{}.tar.gz", image_name))
        }
    };
    
    // Build download URL
    let download_url = format!("{}{}.tar.gz", IMAGE_URL_BASE, image.name);
    
    println!("Checking image: {}", image_name);
    println!("Download URL: {}", download_url);
    println!("Target path: {}", output_path.display());
    println!("Expected SHA256: {}", image.sha256);
    
    // Check if file exists, if so verify SHA256
    if output_path.exists() {
        println!("Local file exists, verifying SHA256...");
        match image_verify_sha256(&output_path, image.sha256) {
            Ok(true) => {
                println!("File verification successful, SHA256 matches, no need to download");
                return Ok(());
            }
            Ok(false) => {
                println!("File verification failed, SHA256 does not match, will re-download");
            }
            Err(e) => {
                println!("Error verifying file: {}, will re-download", e);
            }
        }
    } else {
        println!("Local file does not exist, will start download");
    }
    
    // Ensure target directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    
    println!("Downloading file from {}...", download_url);
    
    // Use reqwest to download the file
    let response = reqwest::get(&download_url).await?;
    if !response.status().is_success() {
        return Err(anyhow!("Failed to download file: HTTP {}", response.status()));
    }
    
    let bytes = response.bytes().await?;
    
    // Write all bytes at once
    let mut file = File::create(&output_path).await?;
    file.write_all(&bytes).await?;
    
    println!("Download completed ({} bytes)", bytes.len());
    
    // Verify downloaded file
    println!("Verifying SHA256 of downloaded file...");
    match image_verify_sha256(&output_path, image.sha256) {
        Ok(true) => {
            println!("Download completed, file verification successful");
        }
        Ok(false) => {
            return Err(anyhow!("Downloaded file SHA256 verification failed"));
        }
        Err(e) => {
            return Err(anyhow!("Error verifying downloaded file: {}", e));
        }
    }
    
    // If extract flag is true, extract the downloaded file
    if extract {
        println!("Extracting downloaded file...");
        
        // Determine extraction output directory
        let extract_dir = output_path.parent()
            .ok_or_else(|| anyhow!("Unable to determine parent directory of downloaded file"))?
            .join(image_name);
        
        // Ensure extraction directory exists
        fs::create_dir_all(&extract_dir)?;
        
        println!("Extracting to: {}", extract_dir.display());
        
        // Use tar command to extract file
        let mut child = Command::new("tar")
            .arg("-xzf")
            .arg(&output_path)
            .arg("-C")
            .arg(&extract_dir)
            .spawn()?;
        
        let status = child.wait()?;
        if !status.success() {
            return Err(anyhow!("Extraction failed, tar exit code: {}", status));
        }
        
        println!("Extraction completed successfully");
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
        .ok_or_else(|| anyhow!("Image not found: {}. Use 'xtask image ls' to view available images", image_name))?;
    
    let temp_dir = env::temp_dir().join("axvisor");
    let tar_file = temp_dir.join(format!("{}.tar.gz", image_name));
    let extract_dir = temp_dir.join(image_name);
    
    let mut removed = false;
    
    // Remove the tar file if it exists
    if tar_file.exists() {
        println!("Removing tar file: {}", tar_file.display());
        fs::remove_file(&tar_file)?;
        removed = true;
    }
    
    // Remove the extracted directory if it exists
    if extract_dir.exists() {
        println!("Removing extracted directory: {}", extract_dir.display());
        fs::remove_dir_all(&extract_dir)?;
        removed = true;
    }
    
    if !removed {
        println!("No files found for image: {}", image_name);
    } else {
        println!("Successfully removed image: {}", image_name);
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
