use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::env;

#[derive(Debug, Deserialize, Serialize)]
pub struct XsiamConfig {
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
    pub instance_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigFile {
    pub xsiam: XsiamConfig,
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

    pub fn load_instance_config(&self, instance_name: &str) -> Result<XsiamConfig> {
        let config_path = format!("{}/config.toml", instance_name);
        
        if !Path::new(&config_path).exists() {
            return Err(anyhow::anyhow!(
                "Instance '{}' not found. Run 'gcgit init --instance {}' first",
                instance_name,
                instance_name
            ));
        }

        let config_content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path))?;

        let config: ConfigFile = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse config file: {}", config_path))?;

        // Expand environment variables in the configuration
        let mut xsiam_config = config.xsiam;
        xsiam_config.fqdn = expand_env_vars(&xsiam_config.fqdn)?;
        xsiam_config.api_key = expand_env_vars(&xsiam_config.api_key)?;
        xsiam_config.api_key_id = expand_env_vars(&xsiam_config.api_key_id)?;

        Ok(xsiam_config)
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
            .with_context(|| format!("Failed to read global config file: {}", config_path))?;

        let config: GlobalConfig = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse global config file: {}", config_path))?;

        Ok(config)
    }

    #[allow(dead_code)]
    pub fn create_instance_structure(&self, instance_name: &str) -> Result<()> {
        // Create the instance directory structure
        fs::create_dir_all(&instance_name)
            .with_context(|| format!("Failed to create instance directory: {}", instance_name))?;
        
        // Create subdirectories for different content types
        let registry = crate::content_types::ContentTypeRegistry::new();
        let subdirs = registry.get_all_types();
        for subdir in &subdirs {
            fs::create_dir_all(format!("{}/{}", instance_name, subdir))
                .with_context(|| format!("Failed to create subdirectory: {}/{}", instance_name, subdir))?;
        }



        // Create config.toml template
        let config_template = r#"[xsiam]
fqdn = "api-myinstance.xdr.au.paloaltonetworks.com"
api_key = "PASTE_YOUR_API_KEY"
api_key_id = "PASTE_YOUR_API_KEY_ID"
instance_name = "{}"
"#;

        let config_content = config_template.replace("{}", instance_name);
        fs::write(format!("{}/config.toml", instance_name), config_content)
            .with_context(|| format!("Failed to create config.toml for instance: {}", instance_name))?;

        Ok(())
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
            .with_context(|| format!("Failed to create instance directory: {}", instance_name))?;

        // Create subdirectories using content type registry
        let registry = crate::content_types::ContentTypeRegistry::new();
        let subdirs = registry.get_all_types();
        for subdir in subdirs {
            let path = format!("{}/{}", instance_name, subdir);
            fs::create_dir_all(&path)
                .with_context(|| format!("Failed to create subdirectory: {}", path))?;
        }

        // Create config.toml template
        let config_template = ConfigFile {
            xsiam: XsiamConfig {
                fqdn: "api-myinstance.xdr.au.paloaltonetworks.com".to_string(),
                api_key: "PASTE_YOUR_API_KEY".to_string(),
                api_key_id: "PASTE_YOUR_API_KEY_ID".to_string(),
                instance_name: instance_name.to_string(),
            },
        };

        let config_content = toml::to_string_pretty(&config_template)
            .context("Failed to serialize config template")?;

        let config_path = format!("{}/config.toml", instance_name);
        fs::write(&config_path, config_content)
            .with_context(|| format!("Failed to write config file: {}", config_path))?;

        Ok(())
    }
}

// Helper function to expand environment variables in strings
fn expand_env_vars(input: &str) -> Result<String> {
    if input.starts_with("${") && input.ends_with("}") {
        let var_name = &input[2..input.len()-1];
        env::var(var_name).with_context(|| format!("Environment variable {} not set", var_name))
    } else {
        Ok(input.to_string())
    }
}
