use std::path::Path;
use std::{fs, path::PathBuf};

use anyhow::{Result, anyhow};

use super::config::ImageConfig;
use super::download::{download_to_path, image_verify_sha256};
use super::registry::ImageRegistry;

///
pub const REGISTRY_FILENAME: &str = "images.toml";

pub struct Storage {
    pub path: PathBuf,
    pub image_registry: ImageRegistry,
}

impl Storage {
    pub fn new(path: PathBuf) -> Result<Self> {
        let registry_filepath = Self::registry_filepath(&path);
        let image_registry = ImageRegistry::load_from_file(&registry_filepath)?;
        Ok(Self {
            path,
            image_registry,
        })
    }

    pub async fn new_with_auto_sync(path: PathBuf, registry: String) -> Result<Self> {
        match Self::new(path.clone()) {
            Ok(storage) => Ok(storage),
            Err(e) => {
                println!("Error while loading local storage: {e}");
                println!("Auto syncing from registry {registry}...");
                Self::new_from_registry(registry, path).await
            }
        }
    }

    pub async fn new_from_registry(registry: String, path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&path).map_err(|e| anyhow!("Failed to create directory: {e}"))?;

        let registry_filepath = Self::registry_filepath(&path);

        download_to_path(&registry, &registry_filepath, Some("Syncing image list")).await?;

        let image_registry = ImageRegistry::load_from_file(&registry_filepath)?;
        println!("Image list saved to {}", registry_filepath.display());

        Ok(Self {
            path,
            image_registry,
        })
    }

    pub async fn new_from_config(config: &ImageConfig) -> Result<Self> {
        if config.auto_sync {
            Self::new_with_auto_sync(config.local_storage.clone(), config.registry.clone()).await
        } else {
            Self::new(config.local_storage.clone())
        }
    }
}

impl Storage {
    pub fn registry_filepath(storage_path: &Path) -> PathBuf {
        storage_path.join(REGISTRY_FILENAME)
    }

    pub fn image_path(storage_path: &Path, image_name: &str) -> PathBuf {
        storage_path.join(format!("{image_name}.tar.gz"))
    }
}

impl Storage {
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

    pub async fn download_image(&self, image_name: &str) -> Result<PathBuf> {
        let output_path = Self::image_path(&self.path, image_name);
        self.download_image_to(image_name, &output_path).await?;
        Ok(output_path)
    }

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
