use anyhow::Context as AnyhowContext;
use ostool::ctx::AppContext;
use std::path::Path;
use toml;

pub struct Context {
    pub ctx: AppContext,
    pub build_config_path: Option<std::path::PathBuf>,
    pub vmconfigs: Vec<String>,
}

impl Context {
    pub fn new() -> Self {
        let workdir = std::env::current_dir().expect("Failed to get current working directory");

        let ctx = AppContext {
            manifest_dir: workdir.clone(),
            workspace_folder: workdir,
            ..Default::default()
        };
        Context {
            ctx,
            build_config_path: None,
            vmconfigs: vec![],
        }
    }

    /// Ensure features are in table format
    pub async fn ensure_features_table_format(&mut self) -> anyhow::Result<()> {
        let build_config_path = self.ctx.workspace_folder.join(".build.toml");

        if !Path::new(&build_config_path).exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&build_config_path).with_context(|| {
            format!(
                "Failed to read .build.toml file: {}",
                build_config_path.display()
            )
        })?;

        let mut config: toml::Value =
            toml::from_str(&content).with_context(|| "Failed to parse .build.toml file")?;

        if let Some(features) = config.get("features") {
            if features.is_table() {
                return Ok(());
            }

            if let Some(features_array) = features.as_array() {
                let mut depend_features = Vec::new();
                let mut self_features = Vec::new();

                for feature in features_array {
                    if let Some(feature_str) = feature.as_str() {
                        if feature_str.contains('/') {
                            depend_features.push(feature_str.to_string());
                        } else {
                            self_features.push(feature_str.to_string());
                        }
                    }
                }

                let mut new_features_table = toml::value::Table::new();
                new_features_table.insert(
                    "depend_features".to_string(),
                    toml::Value::Array(
                        depend_features
                            .into_iter()
                            .map(toml::Value::String)
                            .collect(),
                    ),
                );
                new_features_table.insert(
                    "self_features".to_string(),
                    toml::Value::Array(
                        self_features.into_iter().map(toml::Value::String).collect(),
                    ),
                );

                config["features"] = toml::Value::Table(new_features_table);

                let new_content = toml::to_string_pretty(&config)
                    .with_context(|| "Failed to serialize updated .build.toml")?;

                std::fs::write(&build_config_path, new_content).with_context(|| {
                    format!(
                        "Failed to write updated .build.toml file: {}",
                        build_config_path.display()
                    )
                })?;

                println!("Successfully converted features array to table format");
            }
        }

        Ok(())
    }

    /// Merge depend_features and self_features into a single features array
    pub async fn merge_features_in_build_toml(&mut self) -> anyhow::Result<()> {
        let build_config_path = self.ctx.workspace_folder.join(".build.toml");

        if !Path::new(&build_config_path).exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&build_config_path).with_context(|| {
            format!(
                "Failed to read .build.toml file: {}",
                build_config_path.display()
            )
        })?;

        let mut config: toml::Value =
            toml::from_str(&content).with_context(|| "Failed to parse .build.toml file")?;

        if let Some(features) = config.get("features") {
            if features.is_array() {
                return Ok(());
            }

            if let Some(features_table) = features.as_table() {
                let mut merged_features = Vec::new();

                if let Some(self_features) = features_table
                    .get("self_features")
                    .and_then(|f| f.as_array())
                {
                    for feature in self_features {
                        if let Some(feature_str) = feature.as_str() {
                            merged_features.push(feature_str.to_string());
                        }
                    }
                }

                if let Some(depend_features) = features_table
                    .get("depend_features")
                    .and_then(|f| f.as_array())
                {
                    for feature in depend_features {
                        if let Some(feature_str) = feature.as_str() {
                            merged_features.push(feature_str.to_string());
                        }
                    }
                }

                config["features"] = toml::Value::Array(
                    merged_features
                        .into_iter()
                        .map(toml::Value::String)
                        .collect(),
                );

                let new_content = toml::to_string_pretty(&config)
                    .with_context(|| "Failed to serialize updated .build.toml")?;

                std::fs::write(&build_config_path, new_content).with_context(|| {
                    format!(
                        "Failed to write updated .build.toml file: {}",
                        build_config_path.display()
                    )
                })?;

                println!(
                    "Successfully merged depend_features and self_features into a single features array"
                );
            }
        }

        Ok(())
    }
}
