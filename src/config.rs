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
                "Instance '{}' not found. Run 'gcgit init --instance {}' first",
                instance_name,
                instance_name
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
                    fqdn: expand_env_vars(&data.fqdn)?,
                    api_key: expand_env_vars(&data.api_key)?,
                    api_key_id: expand_env_vars(&data.api_key_id)?,
                });
            }
        }
        
        // Fall back to legacy format for XSIAM only
        if module_id == "xsiam" {
            if let Some(xsiam) = &config.xsiam {
                return Ok(ModuleConfig {
                    enabled: true,
                    fqdn: expand_env_vars(&xsiam.fqdn)?,
                    api_key: expand_env_vars(&xsiam.api_key)?,
                    api_key_id: expand_env_vars(&xsiam.api_key_id)?,
                });
            }
        }
        
        Err(anyhow::anyhow!(
            "Module '{}' not configured in instance '{}'",
            module_id,
            instance_name
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

// Helper function to expand environment variables in strings
fn expand_env_vars(input: &str) -> Result<String> {
    if input.starts_with("${") && input.ends_with("}") {
        let var_name = &input[2..input.len()-1];
        env::var(var_name).with_context(|| format!("Environment variable {var_name} not set"))
    } else {
        Ok(input.to_string())
    }
}
