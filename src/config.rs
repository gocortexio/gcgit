// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::env;
use crate::git_wrapper::GitWrapper;

// Re-export ModuleConfig for public use
pub use crate::modules::ModuleConfig;

// Legacy XSIAM-only config for backwards compatibility
#[derive(Debug, Deserialize, Serialize)]
pub struct XsiamConfig {
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
    pub instance_name: String,
}

// Multi-module configuration format (v2.0+)
#[derive(Debug, Deserialize, Serialize)]
pub struct ModulesConfig {
    pub xsiam: Option<ModuleConfigData>,
    pub appsec: Option<ModuleConfigData>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModuleConfigData {
    pub enabled: Option<bool>,
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
}

// Combined config file format supporting both legacy and multi-module
#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigFile {
    pub instance_name: String,
    
    // Legacy single-module format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xsiam: Option<XsiamConfig>,
    
    // New multi-module format (v2.0+)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<ModulesConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub default_instance: Option<String>,
}

pub struct ConfigManager;

impl ConfigManager {
    pub fn new() -> Self {
        Self
    }

    // Load configuration for a specific module in an instance
    pub fn load_module_config(&self, instance_name: &str, module_id: &str) -> Result<ModuleConfig> {
        let config_path = format!("{instance_name}/config.toml");
        
        if !Path::new(&config_path).exists() {
            return Err(anyhow::anyhow!(
                "Instance '{instance_name}' not found. Run 'gcgit init --instance {instance_name}' first"
            ));
        }

        let config_content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {config_path}"))?;

        let config: ConfigFile = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse config file: {config_path}"))?;

        // Try new multi-module format first
        if let Some(modules) = &config.modules {
            let module_data = match module_id {
                "xsiam" => modules.xsiam.as_ref(),
                "appsec" => modules.appsec.as_ref(),
                _ => None,
            };
            
            if let Some(data) = module_data {
                return Ok(ModuleConfig {
                    enabled: data.enabled.unwrap_or(true),
                    fqdn: resolve_with_fallback(&data.fqdn, "DEMISTO_BASE_URL", "fqdn", module_id)?,
                    api_key: resolve_with_fallback(&data.api_key, "DEMISTO_API_KEY", "api_key", module_id)?,
                    api_key_id: resolve_with_fallback(&data.api_key_id, "XSIAM_AUTH_ID", "api_key_id", module_id)?,
                });
            }
        }
        
        // Fall back to legacy format for XSIAM only
        if module_id == "xsiam" {
            if let Some(xsiam) = &config.xsiam {
                return Ok(ModuleConfig {
                    enabled: true,
                    fqdn: resolve_with_fallback(&xsiam.fqdn, "DEMISTO_BASE_URL", "fqdn", module_id)?,
                    api_key: resolve_with_fallback(&xsiam.api_key, "DEMISTO_API_KEY", "api_key", module_id)?,
                    api_key_id: resolve_with_fallback(&xsiam.api_key_id, "XSIAM_AUTH_ID", "api_key_id", module_id)?,
                });
            }
        }
        
        Err(anyhow::anyhow!(
            "Module '{module_id}' not configured in instance '{instance_name}'"
        ))
    }

    #[allow(dead_code)]
    pub fn load_global_config(&self) -> Result<GlobalConfig> {
        let config_path = ".gcgit/global_config.toml";
        
        if !Path::new(config_path).exists() {
            return Ok(GlobalConfig {
                default_instance: None,
            });
        }

        let config_content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read global config file: {config_path}"))?;

        let config: GlobalConfig = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse global config file: {config_path}"))?;

        Ok(config)
    }



    pub fn create_test_config() -> Result<XsiamConfig> {
        let fqdn = std::env::var("XSIAM_FQDN")
            .context("XSIAM_FQDN environment variable not set")?;
        let api_key = std::env::var("XSIAM_API_KEY")
            .context("XSIAM_API_KEY environment variable not set")?;
        let api_key_id = std::env::var("XSIAM_API_KEY_ID")
            .context("XSIAM_API_KEY_ID environment variable not set")?;

        Ok(XsiamConfig {
            fqdn,
            api_key,
            api_key_id,
            instance_name: "test".to_string(),
        })
    }

    pub fn init_instance(&self, instance_name: &str) -> Result<()> {
        // Create instance directory
        fs::create_dir_all(instance_name)
            .with_context(|| format!("Failed to create instance directory: {instance_name}"))?;

        // Create module subdirectories using module registry
        let module_registry = crate::modules::ModuleRegistry::load();
        for module in module_registry.all_modules() {
            let module_path = format!("{}/{}", instance_name, module.id());
            fs::create_dir_all(&module_path)
                .with_context(|| format!("Failed to create module directory: {module_path}"))?;
            
            // Create content type subdirectories within each module
            for content_type in module.content_types() {
                let content_path = format!("{}/{}", module_path, content_type.name);
                fs::create_dir_all(&content_path)
                    .with_context(|| format!("Failed to create content type directory: {content_path}"))?;
            }
        }

        // Create config.toml template with multi-module format (v2.0+)
        let config_template = ConfigFile {
            instance_name: instance_name.to_string(),
            xsiam: None,  // Use new modules format instead
            modules: Some(ModulesConfig {
                xsiam: Some(ModuleConfigData {
                    enabled: Some(true),
                    fqdn: "${XSIAM_FQDN}".to_string(),
                    api_key: "${XSIAM_API_KEY}".to_string(),
                    api_key_id: "${XSIAM_API_KEY_ID}".to_string(),
                }),
                appsec: Some(ModuleConfigData {
                    enabled: Some(true),
                    fqdn: "${XSIAM_FQDN}".to_string(),  // Often same as XSIAM
                    api_key: "${XSIAM_API_KEY}".to_string(),
                    api_key_id: "${XSIAM_API_KEY_ID}".to_string(),
                }),
            }),
        };

        let config_content = toml::to_string_pretty(&config_template)
            .context("Failed to serialize config template")?;

        let config_path = format!("{instance_name}/config.toml");
        fs::write(&config_path, config_content)
            .with_context(|| format!("Failed to write config file: {config_path}"))?;

        // Initialise git repository
        let _git_repo = GitWrapper::new(instance_name)
            .with_context(|| format!("Failed to initialise git repository in: {instance_name}"))?;

        // Create .gitignore file to exclude config.toml from version control
        let gitignore_path = format!("{instance_name}/.gitignore");
        let gitignore_content = "*.toml\n";
        fs::write(&gitignore_path, gitignore_content)
            .with_context(|| format!("Failed to create .gitignore file: {gitignore_path}"))?;

        Ok(())
    }
}

fn expand_env_vars(input: &str) -> Result<String> {
    if input.starts_with("${") && input.ends_with('}') {
        let var_name = &input[2..input.len()-1];
        match env::var(var_name) {
            Ok(val) if !val.is_empty() => Ok(val),
            _ => Ok(String::new()),
        }
    } else {
        Ok(input.to_string())
    }
}

fn resolve_with_fallback(value: &str, fallback_var: &str, field_label: &str, module_id: &str) -> Result<String> {
    let expanded = expand_env_vars(value)?;
    if !expanded.is_empty() {
        return Ok(expanded);
    }
    match env::var(fallback_var) {
        Ok(val) if !val.is_empty() => {
            let mut resolved = val;
            if field_label == "fqdn" {
                resolved = resolved
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_string();
            }
            eprintln!("[INFO] Using {fallback_var} as fallback for {field_label} (module: {module_id})");
            Ok(resolved)
        }
        _ => Err(anyhow::anyhow!(
            "Configuration field '{field_label}' is empty and fallback variable {fallback_var} is not set (module: {module_id})"
        )),
    }
}
