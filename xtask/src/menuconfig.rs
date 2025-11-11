use crate::ctx::Context;
use anyhow::Context as AnyhowContext;
use cargo_metadata::MetadataCommand;
use std::collections::HashMap;

const DEPEND_PACKAGES: &[&str] = &["axstd", "driver"];

impl Context {
    /// Main menuconfig runner function
    pub async fn run_menuconfig(&mut self) -> anyhow::Result<()> {
        println!("🔧 Starting menuconfig for feature selection...");

        let _ = self.ctx.workspace_folder.join(".build.toml");

        // Get workspace metadata
        let metadata = MetadataCommand::new()
            .exec()
            .with_context(|| "Failed to get workspace metadata")?;

        // Find axvisor package and extract its features
        let axvisor_package = metadata
            .packages
            .iter()
            .find(|pkg| pkg.name == "axvisor")
            .ok_or_else(|| anyhow::anyhow!("axvisor package not found in workspace"))?;

        // Extract axvisor features
        let mut self_features = Vec::new();
        for feature_name in axvisor_package.features.keys() {
            if !feature_name.is_empty() && !feature_name.starts_with("dep:") {
                self_features.push(feature_name.clone());
            }
        }
        self_features.sort();

        println!("Found {} axvisor features:", self_features.len());
        for feature in &self_features {
            println!("  - {}", feature);
        }

        // Create HashMap for package features
        let mut depend_features: HashMap<String, Vec<String>> = HashMap::new();

        // Process each dependency package
        for package_name in DEPEND_PACKAGES {
            if let Some(pkg) = metadata.packages.iter().find(|p| p.name == *package_name) {
                let mut features: Vec<String> = pkg
                    .features
                    .keys()
                    .filter(|&name| !name.is_empty() && !name.starts_with("dep:"))
                    .map(|name| format!("{}/{}", package_name, name))
                    .collect();

                features.sort();

                depend_features.insert(package_name.to_string(), features);
            } else {
                println!("Warning: Package {} not found in workspace", package_name);
            }
        }

        // Process .build.toml file to ensure features are in table format
        self.ensure_features_table_format().await?;

        self.ctx
            .launch_menuconfig_ui(self_features, depend_features)
            .await?;

        // Process .build.toml file to merge depend_features and self_features into a single features array
        self.merge_features_in_build_toml().await?;

        println!("Menuconfig completed successfully!");
        Ok(())
    }
}
