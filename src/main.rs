use clap::{Parser, CommandFactory};
use anyhow::Result;

mod cli;
mod config;
mod git_wrapper;
mod api;
mod parser;
mod error;
mod types;
mod zip_safety;
mod modules;
mod lock;

use cli::{Cli, Commands, ModuleCommands};
use config::ConfigManager;
use git_wrapper::GitWrapper;
use parser::YamlParser;
use modules::ModuleRegistry;
use lock::InstanceLock;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Xsiam { command }) => {
            handle_module_command("xsiam", command).await?;
        }
        Some(Commands::Appsec { command }) => {
            handle_module_command("appsec", command).await?;
        }
        Some(Commands::Init { instance }) => {
            handle_init_command(instance).await?;
        }
        Some(Commands::Status { instance }) => {
            handle_status_command(instance).await?;
        }
        Some(Commands::Deploy { instance: _, message: _, files: _ }) => {
            eprintln!("ERROR: Feature not yet available");
            eprintln!();
            eprintln!("Usage: gcgit deploy [OPTIONS]");
            eprintln!();
            eprintln!("This feature is still under development.");
            eprintln!("Visit https://gocortex.io for updates on feature availability.");
            std::process::exit(1);
        }
        Some(Commands::Validate { instance, files }) => {
            handle_validate_command(instance, files).await?;
        }
        None => {
            // No command provided, show help with version (same as --help)
            let mut cmd = Cli::command();
            cmd.print_long_help().unwrap();
            std::process::exit(0);
        }
    }
    
    Ok(())
}

async fn handle_module_command(module_id: &str, command: ModuleCommands) -> Result<()> {
    // Get the module from registry
    let module_registry = ModuleRegistry::load();
    let module = module_registry.get(module_id)
        .ok_or_else(|| anyhow::anyhow!("Module '{}' not found", module_id))?;
    
    match command {
        ModuleCommands::Push { instance: _ } => {
            let module_upper = module_id.to_uppercase();
            eprintln!("ERROR: Feature not yet available");
            eprintln!();
            eprintln!("Usage: gcgit {module_id} push --instance <NAME>");
            eprintln!();
            eprintln!("Push operations for {module_upper} are still under development.");
            eprintln!("Visit https://gocortex.io for updates on feature availability.");
            std::process::exit(1);
        }
        ModuleCommands::Pull { instance } => {
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            
            // Acquire lock to prevent concurrent operations on the same instance
            let _lock = InstanceLock::acquire(&instance_name)?;
            
            let config_manager = ConfigManager::new();
            let module_config = config_manager.load_module_config(&instance_name, module_id)?;
            
            // Check if module is enabled
            if !module_config.enabled {
                println!("Module '{module_id}' is disabled in instance '{instance_name}'. Enable it in config.toml to use this command.");
                return Ok(());
            }
            
            let module_client = api::ModuleClient::new(module_config, module.base_api_path());
            let yaml_parser = YamlParser::new();
            
            // Pull each content type defined in the module
            let content_types = module.content_types();
            
            let mut _total_pulled = 0;
            let mut pulled_files = Vec::new();
            
            for content_def in content_types {
                println!("Pulling {}...", content_def.name);
                match module_client.pull_content_type(&content_def).await {
                    Ok(objects) => {
                        println!("  Found {} {}(s)", objects.len(), content_def.name);
                        for object in objects {
                            // Create filename from name, falling back to ID if name is empty
                            let filename = if let Some(name) = &object.name {
                                if name.trim().is_empty() {
                                    format!("{}_id_{}", content_def.name.trim_end_matches('s'), object.id)
                                } else {
                                    name.replace(" ", "_").replace("/", "_").replace("\\", "_")
                                }
                            } else {
                                format!("{}_id_{}", content_def.name.trim_end_matches('s'), object.id)
                            };
                            
                            // NEW directory structure: instance/module_id/content_type/filename.yaml
                            let file_path = format!("{}/{}/{}/{}.yaml", instance_name, module_id, content_def.name, filename);
                            yaml_parser.write_file(&file_path, &object)?;
                            println!("  Pulled: {file_path}");
                            // Store relative path for Git operations (relative to instance directory)
                            let relative_path = format!("{}/{}/{}.yaml", module_id, content_def.name, filename);
                            pulled_files.push(relative_path);
                            _total_pulled += 1;
                        }
                    }
                    Err(e) => {
                        println!("  WARNING: Failed to pull {} - {}", content_def.name, e);
                        println!("  (This endpoint may not be available on your instance)");
                    }
                }
            }
            
            // Auto-commit pulled changes using Git's native change detection
            if !pulled_files.is_empty() {
                println!("\nProcessing pulled files for Git repository...");
                
                match GitWrapper::new_for_instance(&instance_name) {
                    Ok(git_wrapper) => {
                        // Use Git's native change detection - much faster than API calls
                        match git_wrapper.has_changes_after_add(&pulled_files) {
                            Ok((true, changed_count, changed_files)) => {
                                // Create descriptive commit message with changed files
                                let changed_file_names: Vec<String> = changed_files.iter()
                                    .map(|path| {
                                        // Extract just the filename from the path for readability
                                        if let Some(filename) = path.split('/').next_back() {
                                            filename.replace(".yaml", "")
                                        } else {
                                            path.clone()
                                        }
                                    })
                                    .collect();
                                
                                let module_upper = module_id.to_uppercase();
                                let commit_message = if changed_count == 1 {
                                    format!("Auto-commit: Updated {} from {}", changed_file_names[0], module_upper)
                                } else if changed_count <= 3 {
                                    format!("Auto-commit: Updated {} from {}", changed_file_names.join(", "), module_upper)
                                } else {
                                    format!("Auto-commit: Updated {} files from {} ({})", changed_count, module_upper, changed_file_names[..2].join(", "))
                                };
                                
                                if let Err(e) = git_wrapper.commit(&commit_message) {
                                    println!("Warning: Failed to commit changes: {e}");
                                } else {
                                    let file_word = if changed_count == 1 { "file" } else { "files" };
                                    println!("Successfully processed {} pulled files to instance Git repository", pulled_files.len());
                                    println!("  {changed_count} {file_word} actually changed and committed");
                                }
                            }
                            Ok((false, _, _)) => {
                                println!("Successfully processed {} pulled files to instance Git repository", pulled_files.len());
                                println!("  No Git changes detected - objects serialise to identical YAML");
                            }
                            Err(e) => {
                                println!("Warning: Failed to check for changes: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        println!("Warning: Failed to initialise Git repository for instance: {e}");
                    }
                }
            }
        }
        ModuleCommands::Diff { instance } => {
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            
            let config_manager = ConfigManager::new();
            let module_config = config_manager.load_module_config(&instance_name, module_id)?;
            
            // Check if module is enabled
            if !module_config.enabled {
                println!("Module '{module_id}' is disabled in instance '{instance_name}'. Enable it in config.toml to use this command.");
                return Ok(());
            }
            
            let module_client = api::ModuleClient::new(module_config, module.base_api_path());
            let yaml_parser = YamlParser::new();
            
            // Get local files from the module-specific directory
            let module_dir = format!("{instance_name}/{module_id}");
            
            // Get content type names from the module definition
            let content_type_names: Vec<&str> = module.content_types()
                .iter()
                .map(|ct| ct.name)
                .collect();
            
            let local_files = yaml_parser.get_local_files(&module_dir, &content_type_names)?;
            
            if local_files.is_empty() {
                println!("No local YAML files found for module '{module_id}' in instance '{instance_name}'");
                println!("Run 'gcgit {module_id} pull --instance {instance_name}' to fetch configurations first");
                return Ok(());
            }
            
            let mut differences_found = false;
            
            // Get content type definitions once (needed for lifetime)
            let content_types = module.content_types();
            
            for file_path in local_files {
                let local_content = yaml_parser.parse_file(&file_path)?;
                
                // Find the ContentTypeDefinition for this content type
                let content_def = content_types
                    .iter()
                    .find(|ct| ct.name == local_content.content_type)
                    .ok_or_else(|| anyhow::anyhow!("Content type '{}' not found in module definition", local_content.content_type))?;
                
                match module_client.get_object_by_id(content_def, &local_content.id).await {
                    Ok(remote_content) => {
                        // Use logical comparison (excludes metadata for accurate functional comparison)
                        match yaml_parser.objects_are_logically_equal(&local_content, &remote_content) {
                            Ok(are_equal) => {
                                if !are_equal {
                                    differences_found = true;
                                    println!("DIFF: {file_path} (local differs from remote)");
                                    
                                    // Show a detailed summary of what actually differs
                                    show_object_differences(&yaml_parser, &local_content, &remote_content);
                                }
                            }
                            Err(e) => {
                                differences_found = true;
                                println!("WARNING: {file_path} (comparison failed: {e})");
                                // Fallback to struct comparison if serialisation fails
                                if local_content != remote_content {
                                    println!("DIFF: {file_path} (local differs from remote - fallback comparison)");
                                }
                            }
                        }
                    }
                    Err(_) => {
                        differences_found = true;
                        println!("NEW: {file_path} (exists locally but not remotely)");
                    }
                }
            }
            
            // Provide feedback when no differences are found
            if !differences_found {
                println!("No differences detected - local YAML files match remote {} objects", module_id.to_uppercase());
            }
        }
        ModuleCommands::Test { instance } => {
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            
            let config_manager = ConfigManager::new();
            let module_config = match config_manager.load_module_config(&instance_name, module_id) {
                Ok(config) => {
                    // Check if module is enabled
                    if !config.enabled {
                        println!("Module '{module_id}' is disabled in instance '{instance_name}'. Enable it in config.toml to use this command.");
                        return Ok(());
                    }
                    config
                },
                Err(_) => {
                    println!("Module '{module_id}' configuration not found for instance '{instance_name}'. Trying environment variables...");
                    
                    // Fallback to environment variables if instance config doesn't exist
                    match ConfigManager::create_test_config() {
                        Ok(config) => {
                            // Convert XsiamConfig to ModuleConfig
                            crate::config::ModuleConfig {
                                enabled: true,
                                fqdn: config.fqdn,
                                api_key: config.api_key,
                                api_key_id: config.api_key_id,
                            }
                        }
                        Err(e) => {
                            println!("ERROR: Configuration error: {e}");
                            println!("\nTo fix this, either:");
                            println!("  1. Create an instance: gcgit init --instance {instance_name}");
                            println!("  2. Set environment variables: XSIAM_FQDN, XSIAM_API_KEY, XSIAM_API_KEY_ID");
                            return Ok(());
                        }
                    }
                }
            };
            
            let module_client = api::ModuleClient::new(module_config, module.base_api_path());
            
            println!("Testing {} API connectivity...\n", module_id.to_uppercase());
            
            // Test connectivity
            match module_client.test_connectivity().await {
                Ok(_) => {
                    println!("API connectivity test successful");
                    
                    // Test each content type endpoint
                    let content_types = module.content_types();
                    let mut successful_endpoints = 0;
                    let total_endpoints = content_types.len();
                    
                    for content_def in content_types {
                        print!("Testing {:<25} ", format!("{}:", content_def.name));
                        
                        match module_client.pull_content_type(&content_def).await {
                            Ok(objects) => {
                                println!("OK ({} items)", objects.len());
                                successful_endpoints += 1;
                            }
                            Err(e) => {
                                println!("FAILED: {e}");
                            }
                        }
                    }
                    
                    println!("\n{successful_endpoints}/{total_endpoints} endpoints available");
                    
                    if successful_endpoints == total_endpoints {
                        println!("All {} module endpoints are operational", module_id.to_uppercase());
                    } else if successful_endpoints > 0 {
                        println!("WARNING: Some endpoints unavailable (this may be normal depending on your licence)");
                    } else {
                        println!("ERROR: No endpoints available - check your configuration");
                    }
                }
                Err(e) => {
                    println!("\nERROR: API connectivity test failed: {e}");
                }
            }
        }
        ModuleCommands::Delete { instance: _, content_type: _, id: _ } => {
            let module_upper = module_id.to_uppercase();
            eprintln!("ERROR: Feature not yet available");
            eprintln!();
            eprintln!("Usage: gcgit {module_id} delete --instance <NAME> --content-type <TYPE> --id <ID>");
            eprintln!();
            eprintln!("Delete operations for {module_upper} are still under development.");
            eprintln!("Visit https://gocortex.io for updates on feature availability.");
            std::process::exit(1);
        }
    }
    
    Ok(())
}

async fn handle_init_command(instance: String) -> Result<()> {
    let config_manager = ConfigManager::new();
    config_manager.init_instance(&instance)?;
    
    println!("Initialised instance: {instance}");
    println!("Please edit {instance}/config.toml with your API credentials");
    println!("  Configure modules.xsiam for XSIAM platform access");
    println!("  Configure modules.appsec for Application Security platform access");
    
    Ok(())
}

async fn handle_status_command(instance: Option<String>) -> Result<()> {
    let config_manager = ConfigManager::new();
    
    match instance {
        Some(instance_name) => {
            println!("Status for instance: {instance_name}");
            show_instance_status(&config_manager, &instance_name).await?;
        }
        None => {
            println!("Status for all instances:");
            // Get all instance directories
            let instances = get_all_instances()?;
            for instance_name in instances {
                println!("\n=== {instance_name} ===");
                show_instance_status(&config_manager, &instance_name).await?;
            }
        }
    }
    
    Ok(())
}

async fn handle_validate_command(instance: Option<String>, files: Vec<String>) -> Result<()> {
    let yaml_parser = YamlParser::new();
    let module_registry = ModuleRegistry::load();
    
    // Collect all content type names from all modules for validation
    let all_content_types: Vec<&str> = module_registry.all_modules()
        .iter()
        .flat_map(|module| module.content_types())
        .map(|ct| ct.name)
        .collect();
    
    // Determine files to validate
    let files_to_validate = if !files.is_empty() {
        files
    } else if let Some(instance_name) = &instance {
        // Get all YAML files in the specified instance across all modules
        let mut instance_files = Vec::new();
        for module in module_registry.all_modules() {
            let module_dir = format!("{}/{}", instance_name, module.id());
            if let Ok(files) = yaml_parser.get_local_files(&module_dir, &all_content_types) {
                instance_files.extend(files);
            }
        }
        instance_files
    } else {
        // Get all YAML files in all instances
        let instances = get_all_instances()?;
        let mut all_files = Vec::new();
        for inst in instances {
            for module in module_registry.all_modules() {
                let module_dir = format!("{}/{}", inst, module.id());
                if let Ok(files) = yaml_parser.get_local_files(&module_dir, &all_content_types) {
                    all_files.extend(files);
                }
            }
        }
        all_files
    };
    
    if files_to_validate.is_empty() {
        println!("No YAML files found to validate");
        return Ok(());
    }
    
    println!("Validating {} files...", files_to_validate.len());
    let mut validation_errors = 0;
    
    for file_path in files_to_validate {
        print!("  Checking {file_path}... ");
        
        match yaml_parser.parse_file(&file_path) {
            Ok(xsiam_object) => {
                // Validate content type is supported by checking against all registered modules
                if all_content_types.contains(&xsiam_object.content_type.as_str()) {
                    println!("Valid");
                } else {
                    println!("INVALID: Unsupported content type: {}", xsiam_object.content_type);
                    validation_errors += 1;
                }
            }
            Err(e) => {
                println!("ERROR: {e}");
                validation_errors += 1;
            }
        }
    }
    
    if validation_errors > 0 {
        println!("\n{validation_errors} validation errors found");
        return Err(anyhow::anyhow!("Validation failed"));
    } else {
        println!("\nAll files are valid");
    }
    
    Ok(())
}

async fn show_instance_status(config_manager: &ConfigManager, instance_name: &str) -> Result<()> {
    // Check if instance exists
    if !std::path::Path::new(instance_name).exists() {
        println!("  Instance '{instance_name}' not found");
        return Ok(());
    }
    
    // Git status for this instance (using instance-specific git repo)
    match GitWrapper::new_for_instance(instance_name) {
        Ok(git_wrapper) => {
            let modified_files = git_wrapper.get_modified_files_in_current_repo()?;
            
            if modified_files.is_empty() {
                println!("  Git: No modified files");
            } else {
                println!("  Git: {} modified files", modified_files.len());
                for file in &modified_files {
                    println!("    - {file}");
                }
            }
        }
        Err(_) => {
            println!("  Git: No repository (run gcgit pull to initialise)");
        }
    }
    
    // Module connectivity status - check all enabled modules dynamically
    let module_registry = crate::modules::ModuleRegistry::load();
    for module in module_registry.all_modules() {
        let module_id = module.id();
        
        match config_manager.load_module_config(instance_name, module_id) {
            Ok(module_config) => {
                if module_config.enabled {
                    let module_client = api::ModuleClient::new(module_config, module.base_api_path());
                    match module_client.test_connectivity().await {
                        Ok(_) => println!("  {}: Connected", module_id.to_uppercase()),
                        Err(e) => println!("  {}: Connection failed - {e}", module_id.to_uppercase()),
                    }
                } else {
                    println!("  {}: Disabled", module_id.to_uppercase());
                }
            }
            Err(_) => {
                // Module not configured - skip silently
            }
        }
    }
    
    Ok(())
}

fn get_all_instances() -> Result<Vec<String>> {
    use std::fs;
    
    let mut instances = Vec::new();
    
    for entry in fs::read_dir(".")? {
        let entry = entry?;
        let path = entry.path();
        
        if path.is_dir() {
            if let Some(dir_name) = path.file_name() {
                if let Some(dir_str) = dir_name.to_str() {
                    // Check if this looks like an instance directory (has config.toml)
                    let config_path = path.join("config.toml");
                    if config_path.exists() {
                        instances.push(dir_str.to_string());
                    }
                }
            }
        }
    }
    
    Ok(instances)
}

use crate::types::XsiamObject;

/// Display a detailed summary of differences between local and remote objects
fn show_object_differences(yaml_parser: &YamlParser, local: &XsiamObject, remote: &XsiamObject) {
        let mut differences = Vec::new();
        
        // Check basic field differences
        if local.id != remote.id {
            differences.push(format!("  → ID: '{}' → '{}'", local.id, remote.id));
        }
        if local.name != remote.name {
            let local_name = local.name.as_deref().unwrap_or(&local.id);
            let remote_name = remote.name.as_deref().unwrap_or(&remote.id);
            differences.push(format!("  → Name: '{}' → '{}'", 
                truncate_string(local_name, 30), 
                truncate_string(remote_name, 30)));
        }
        if local.description != remote.description {
            differences.push(format!("  → Description: {} chars → {} chars", 
                local.description.len(), remote.description.len()));
        }
        if local.content_type != remote.content_type {
            differences.push(format!("  → Type: '{}' → '{}'", local.content_type, remote.content_type));
        }
        
        // Check content differences
        let content_diffs = analyze_content_differences(&local.content, &remote.content);
        differences.extend(content_diffs);
        
        // Display differences with helpful formatting
        if differences.is_empty() {
            println!("  → No functional differences detected (metadata-only changes)");
        } else {
            for diff in &differences {
                println!("{diff}");
            }
            
            // Show helpful action suggestions
            if differences.len() > 1 {
                println!("  → {} changes detected", differences.len());
            }
            
            // Check if YAML serialisation differs
            if let (Ok(local_yaml), Ok(remote_yaml)) = (
                yaml_parser.serialize_object_deterministically(local),
                yaml_parser.serialize_object_deterministically(remote)
            ) {
                if local_yaml != remote_yaml {
                    println!("  → File content will change on next pull");
                } else {
                    println!("  → File content unchanged (structural differences only)");
                }
            }
        }
    }

/// Analyze differences in content HashMap
fn analyze_content_differences(local: &std::collections::HashMap<String, serde_json::Value>, remote: &std::collections::HashMap<String, serde_json::Value>) -> Vec<String> {
        let mut differences = Vec::new();
        
        // Find keys that exist in both
        let mut all_keys: std::collections::HashSet<String> = local.keys().cloned().collect();
        all_keys.extend(remote.keys().cloned());
        
        let mut modified_keys = Vec::new();
        let mut added_keys = Vec::new();
        let mut removed_keys = Vec::new();
        
        for key in all_keys {
            match (local.get(&key), remote.get(&key)) {
                (Some(local_val), Some(remote_val)) => {
                    if local_val != remote_val {
                        modified_keys.push(key);
                    }
                }
                (None, Some(_)) => added_keys.push(key),
                (Some(_), None) => removed_keys.push(key),
                (None, None) => {} // Shouldn't happen
            }
        }
        
        // Format the differences with helpful summaries
        if !added_keys.is_empty() {
            if added_keys.len() <= 3 {
                differences.push(format!("  → Added fields: {}", added_keys.join(", ")));
            } else {
                differences.push(format!("  → Added {} new fields: {}, ...", 
                    added_keys.len(), added_keys[..2].join(", ")));
            }
        }
        
        if !removed_keys.is_empty() {
            if removed_keys.len() <= 3 {
                differences.push(format!("  → Removed fields: {}", removed_keys.join(", ")));
            } else {
                differences.push(format!("  → Removed {} fields: {}, ...", 
                    removed_keys.len(), removed_keys[..2].join(", ")));
            }
        }
        
        if !modified_keys.is_empty() {
            if modified_keys.len() <= 3 {
                differences.push(format!("  → Modified fields: {}", modified_keys.join(", ")));
            } else {
                differences.push(format!("  → Modified {} fields: {}, ...", 
                    modified_keys.len(), modified_keys[..2].join(", ")));
            }
        }
        
        differences
}

/// Truncate string for display purposes
fn truncate_string(s: &str, max_len: usize) -> String {
        if s.len() <= max_len {
            s.to_string()
        } else {
            format!("{}...", &s[..max_len-3])
        }
}
