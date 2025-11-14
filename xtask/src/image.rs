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
use std::path::{Path};
use std::process::Command;
use std::fs;
use std::env;

/// Base URL for downloading images
const IMAGE_URL_BASE: &str = "https://github.com/arceos-hypervisor/axvisor-guest/releases/download/v0.0.16/";

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
        sha256: "29b776507716974b4db24cb00e1511c3d35ce90807826c7136b4761843da2eec",
        arch: "aarch64",
    };
    
    pub const EVM3588_LINUX: Self = Self {
        name: "evm3588_linux",
        description: "Linux for EVM3588 development board",
        sha256: "ae727f1f5febbd205e7a7804a08dd388a030d140662f07704e29a8c51a1baec8",
        arch: "aarch64",
    };
    
    pub const ORANGEPI_ARCEOS: Self = Self {
        name: "orangepi_arceos",
        description: "ArceOS for Orange Pi development board",
        sha256: "9533e22c14d6a17d663aa6afcd399711afeb29f914eb47ce6a0469e59fd5c600",
        arch: "aarch64",
    };
    
    pub const ORANGEPI_LINUX: Self = Self {
        name: "orangepi_linux",
        description: "Linux for Orange Pi development board",
        sha256: "22b96c1f582ab75b55ec86230d2403a0254a4ecf75f2bd04539824120276d76e",
        arch: "aarch64",
    };
    
    pub const PHYTIUMPI_ARCEOS: Self = Self {
        name: "phytiumpi_arceos",
        description: "ArceOS for Phytium Pi development board",
        sha256: "0aa08692cecf8fc19851beca119888ad315b3fa809dc31b39ce23b2f4128810f",
        arch: "aarch64",
    };
    
    pub const PHYTIUMPI_LINUX: Self = Self {
        name: "phytiumpi_linux",
        description: "Linux for Phytium Pi development board",
        sha256: "3f06e9f3322555203e1c0a77a43acf87b3a9e5bc7e0a72f9182b3338fac0468d",
        arch: "aarch64",
    };
    
    pub const QEMU_ARCEOS_AARCH64: Self = Self {
        name: "qemu_arceos_aarch64",
        description: "ArceOS for QEMU aarch64 virtualization",
        sha256: "f34b94fb35cf3e2d608018672d5b081155741b70ae67988e90f6fea283a97f0e",
        arch: "aarch64",
    };
    
    pub const QEMU_ARCEOS_RISCV64: Self = Self {
        name: "qemu_arceos_riscv64",
        description: "ArceOS for QEMU riscv64 virtualization",
        sha256: "5c69be5cad65f7258db4130ba64cd70c8127aec94e384df619d34ce1ef27af00",
        arch: "riscv64",
    };
    
    pub const QEMU_ARCEOS_X86_64: Self = Self {
        name: "qemu_arceos_x86_64",
        description: "ArceOS for QEMU x86_64 virtualization",
        sha256: "b5ea91fc5bd34f8a12f63c66adc23c2d5a7bc96e1b10ea415f7a526a5301605f",
        arch: "x86_64",
    };
    
    pub const QEMU_LINUX_AARCH64: Self = Self {
        name: "qemu_linux_aarch64",
        description: "Linux for QEMU aarch64 virtualization",
        sha256: "a4c878633da6655acf0cc22565e8dbf05b370260ed802519897f15001f30da75",
        arch: "aarch64",
    };
    
    pub const QEMU_LINUX_RISCV64: Self = Self {
        name: "qemu_linux_riscv64",
        description: "Linux for QEMU riscv64 virtualization",
        sha256: "b198759c1c99b5edb959b26ce63b77165d653227fb9451941b0bb46f0c9afc74",
        arch: "riscv64",
    };
    
    pub const QEMU_LINUX_X86_64: Self = Self {
        name: "qemu_linux_x86_64",
        description: "Linux for QEMU x86_64 virtualization",
        sha256: "bd036a6fa47c5c345ed510bae44c87ad9534a33cb08ff0a142e0b45840453922",
        arch: "x86_64",
    };
    
    pub const ROC_RK3568_PC_ARCEOS: Self = Self {
        name: "roc-rk3568-pc_arceos",
        description: "ArceOS for ROC-RK3568-PC development board",
        sha256: "0a239035da686e06bce68420389e9a232cae8fad7282d3f2ca4e9adead0794cc",
        arch: "aarch64",
    };
    
    pub const ROC_RK3568_PC_LINUX: Self = Self {
        name: "roc-rk3568-pc_linux",
        description: "Linux for ROC-RK3568-PC development board",
        sha256: "b2c837b864e72cc3ec1bbd4bfb9b04fa55f11109b99aee20a5dcf10d18c39678",
        arch: "aarch64",
    };
    
    pub const TAC_E400_PLC_ARCEOS: Self = Self {
        name: "tac-e400-plc_arceos",
        description: "ArceOS for TAC-E400-PLC industrial control board",
        sha256: "1dc8fd5aeced6a85fca144511e69f7845a27b2337e60cf282807df91bb9445fd",
        arch: "aarch64",
    };
    
    pub const TAC_E400_PLC_LINUX: Self = Self {
        name: "tac-e400-plc_linux",
        description: "Linux for TAC-E400-PLC industrial control board",
        sha256: "78de5ce8729b6d342172de7e86cabf0b94fb7b993c1f66564eb6c66de121c17e",
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
    let output = Command::new("sha256sum")
        .arg(file_path)
        .output()?;
    
    if !output.status.success() {
        return Err(anyhow!("Failed to calculate SHA256"));
    }
    
    let stdout = String::from_utf8(output.stdout)?;
    let actual_sha256 = stdout.split_whitespace().next()
        .ok_or_else(|| anyhow!("Unable to parse SHA256 output"))?;
    
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
/// // Download the evm3588_arceos image to the ./images directory without extracting
/// xtask image download evm3588_arceos --output-dir ./images --no-extract
/// ```
fn image_download(image_name: &str, output_dir: Option<String>, extract: bool) -> Result<()> {
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
    
    let mut child = Command::new("curl")
        .arg("-L") // Follow redirects
        .arg("-o")
        .arg(&output_path)
        .arg(&download_url)
        .spawn()?;
    
    let status = child.wait()?;
    if !status.success() {
        return Err(anyhow!("Download failed, curl exit code: {}", status));
    }
    
    println!("Download completed");
    
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
/// xtask image download evm3588_arceos --output-dir ./images --no-extract
/// xtask image rm evm3588_arceos
/// ```
pub fn run_image(args: ImageArgs) -> Result<()> {
    match args.command {
        ImageCommands::Ls => {
            image_list()?;
        }
        ImageCommands::Download { image_name, output_dir, extract } => {
            image_download(&image_name, output_dir, extract.unwrap_or(true))?;
        }
        ImageCommands::Rm { image_name } => {
            image_remove(&image_name)?;
        }
    }
    
    Ok(())
}
