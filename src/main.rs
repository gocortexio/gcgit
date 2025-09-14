use clap::{Parser, CommandFactory};
use anyhow::Result;

mod cli;
mod config;
mod content_types;
mod git_wrapper;
mod api;
mod parser;
mod error;
mod types;

use cli::{Cli, Commands, XsiamCommands};
use config::ConfigManager;
use content_types::ContentTypeRegistry;
use git_wrapper::GitWrapper;
use api::XsiamClient;
use parser::YamlParser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Xsiam { command }) => {
            handle_xsiam_command(command).await?;
        }
        Some(Commands::Init { instance }) => {
            handle_init_command(instance).await?;
        }
        Some(Commands::Status { instance }) => {
            handle_status_command(instance).await?;
        }
        Some(Commands::Deploy { instance: _, message: _, files: _ }) => {
            println!("This command did not run as the feature is still under development, keep an eye on https://gocortex.io for updates");
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

async fn handle_xsiam_command(command: XsiamCommands) -> Result<()> {
    match command {
        XsiamCommands::Push { instance: _ } => {
            println!("This command did not run as the feature is still under development, keep an eye on https://gocortex.io for updates");
            return Ok(());
        }
        XsiamCommands::Pull { instance } => {
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            
            let config_manager = ConfigManager::new();
            let instance_config = config_manager.load_instance_config(&instance_name)?;
            
            let xsiam_client = XsiamClient::new(instance_config);
            let yaml_parser = YamlParser::new();
            
            // Pull each content type, handling errors gracefully
            let registry = ContentTypeRegistry::new();
            let content_types = registry.get_all_types();
            
            let mut _total_pulled = 0;
            let mut pulled_files = Vec::new();
            
            for content_type in content_types {
                println!("Pulling {}...", content_type);
                match xsiam_client.get_objects(content_type).await {
                    Ok(objects) => {
                        println!("  Found {} {}(s)", objects.len(), content_type);
                        for object in objects {
                            // Create filename from name, falling back to ID if name is empty
                            let filename = if object.name.trim().is_empty() {
                                format!("{}_id_{}", content_type.trim_end_matches('s'), object.id)
                            } else {
                                object.name.replace(" ", "_").replace("/", "_").replace("\\", "_")
                            };
                            
                            let file_path = format!("{}/{}/{}.yaml", instance_name, content_type, filename);
                            yaml_parser.write_file(&file_path, &object)?;
                            println!("  Pulled: {}", file_path);
                            // Store relative path for Git operations (without instance prefix)
                            let relative_path = format!("{}/{}.yaml", content_type, filename);
                            pulled_files.push(relative_path);
                            _total_pulled += 1;
                        }
                    }
                    Err(e) => {
                        println!("  WARNING: Failed to pull {} - {}", content_type, e);
                        println!("  (This endpoint may not be available on your XSIAM instance)");
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
                                        if let Some(filename) = path.split('/').last() {
                                            filename.replace(".yaml", "")
                                        } else {
                                            path.clone()
                                        }
                                    })
                                    .collect();
                                
                                let commit_message = if changed_count == 1 {
                                    format!("Auto-commit: Updated {} from XSIAM", changed_file_names[0])
                                } else if changed_count <= 3 {
                                    format!("Auto-commit: Updated {} from XSIAM", changed_file_names.join(", "))
                                } else {
                                    format!("Auto-commit: Updated {} files from XSIAM ({})", changed_count, changed_file_names[..2].join(", "))
                                };
                                
                                if let Err(e) = git_wrapper.commit(&commit_message) {
                                    println!("Warning: Failed to commit changes: {}", e);
                                } else {
                                    let file_word = if changed_count == 1 { "file" } else { "files" };
                                    println!("✓ Successfully processed {} pulled files to instance Git repository", pulled_files.len());
                                    println!("  {} {} actually changed and committed", changed_count, file_word);
                                }
                            }
                            Ok((false, _, _)) => {
                                println!("✓ Successfully processed {} pulled files to instance Git repository", pulled_files.len());
                                println!("  No Git changes detected - XSIAM objects serialize to identical YAML");
                            }
                            Err(e) => {
                                println!("Warning: Failed to check for changes: {}", e);
                            }
                        }

                    }
                    Err(e) => {
                        println!("Warning: Failed to initialise Git repository for instance: {}", e);
                    }
                }
            }
        }
        XsiamCommands::Diff { instance } => {
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            
            let config_manager = ConfigManager::new();
            let instance_config = config_manager.load_instance_config(&instance_name)?;
            
            let xsiam_client = XsiamClient::new(instance_config);
            let yaml_parser = YamlParser::new();
            
            let local_files = yaml_parser.get_local_files(&instance_name)?;
            
            if local_files.is_empty() {
                println!("No local YAML files found in instance '{}'", instance_name);
                println!("Run 'gcgit xsiam pull --instance {}' to fetch configurations first", instance_name);
                return Ok(());
            }
            
            let mut differences_found = false;
            
            for file_path in local_files {
                let local_content = yaml_parser.parse_file(&file_path)?;
                
                match xsiam_client.get_object_by_id(&local_content.content_type, &local_content.id).await {
                    Ok(remote_content) => {
                        // Use logical comparison (excludes metadata for accurate functional comparison)
                        match yaml_parser.objects_are_logically_equal(&local_content, &remote_content) {
                            Ok(are_equal) => {
                                if !are_equal {
                                    differences_found = true;
                                    println!("DIFF: {} (local differs from remote)", file_path);
                                    
                                    // Show a detailed summary of what actually differs
                                    show_object_differences(&yaml_parser, &local_content, &remote_content);
                                }
                            }
                            Err(e) => {
                                differences_found = true;
                                println!("WARNING: {} (comparison failed: {})", file_path, e);
                                // Fallback to struct comparison if serialisation fails
                                if local_content != remote_content {
                                    println!("DIFF: {} (local differs from remote - fallback comparison)", file_path);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        differences_found = true;
                        println!("NEW: {} (exists locally but not remotely)", file_path);
                    }
                }
            }
            
            // Provide feedback when no differences are found
            if !differences_found {
                println!("✓ No differences detected - local YAML files match remote XSIAM objects");
            }
        }
        XsiamCommands::Test { instance } => {
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            
            let config_manager = ConfigManager::new();
            let test_config = match config_manager.load_instance_config(&instance_name) {
                Ok(config) => config,
                Err(_) => {
                    println!("Instance '{}' not found. Trying environment variables...", instance_name);
                    
                    // Fallback to environment variables if instance config doesn't exist
                    match ConfigManager::create_test_config() {
                        Ok(config) => config,
                        Err(e) => {
                            println!("✗ Configuration error: {}", e);
                            println!("\nTo fix this, either:");
                            println!("  1. Create an instance: gcgit init --instance {}", instance_name);
                            println!("  2. Set environment variables: XSIAM_FQDN, XSIAM_API_KEY, XSIAM_API_KEY_ID");
                            return Ok(());
                        }
                    }
                }
            };
            
            let xsiam_client = XsiamClient::new(test_config);
            
            // Run comprehensive endpoint testing
            match xsiam_client.test_all_endpoints().await {
                Ok(_) => {
                    println!("\n✓ Endpoint testing completed successfully");
                }
                Err(e) => {
                    println!("\n✗ Endpoint testing failed: {}", e);
                }
            }
        }
        XsiamCommands::Delete { instance: _, content_type: _, id: _ } => {
            println!("This command did not run as the feature is still under development, keep an eye on https://gocortex.io for updates");
            return Ok(());
        }
    }
    
    Ok(())
}

async fn handle_init_command(instance: String) -> Result<()> {
    let config_manager = ConfigManager::new();
    config_manager.init_instance(&instance)?;
    
    println!("Initialised instance: {}", instance);
    println!("Please edit {}/config.toml with your XSIAM API credentials", instance);
    
    Ok(())
}

async fn handle_status_command(instance: Option<String>) -> Result<()> {
    let config_manager = ConfigManager::new();
    
    match instance {
        Some(instance_name) => {
            println!("Status for instance: {}", instance_name);
            show_instance_status(&config_manager, &instance_name).await?;
        }
        None => {
            println!("Status for all instances:");
            // Get all instance directories
            let instances = get_all_instances()?;
            for instance_name in instances {
                println!("\n=== {} ===", instance_name);
                show_instance_status(&config_manager, &instance_name).await?;
            }
        }
    }
    
    Ok(())
}

async fn handle_validate_command(instance: Option<String>, files: Vec<String>) -> Result<()> {
    let yaml_parser = YamlParser::new();
    let registry = ContentTypeRegistry::new();
    
    // Determine files to validate
    let files_to_validate = if !files.is_empty() {
        files
    } else if let Some(instance_name) = &instance {
        // Get all YAML files in the specified instance
        yaml_parser.get_local_files(instance_name)?
    } else {
        // Get all YAML files in all instances
        let instances = get_all_instances()?;
        let mut all_files = Vec::new();
        for inst in instances {
            all_files.extend(yaml_parser.get_local_files(&inst)?);
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
        print!("  Checking {}... ", file_path);
        
        match yaml_parser.parse_file(&file_path) {
            Ok(xsiam_object) => {
                // Validate content type is supported
                match registry.validate_content_type(&xsiam_object.content_type) {
                    Ok(_) => {
                        // Additional validation could go here (e.g., required fields)
                        println!("✓ Valid");
                    }
                    Err(e) => {
                        println!("✗ {}", e);
                        validation_errors += 1;
                    }
                }
            }
            Err(e) => {
                println!("✗ {}", e);
                validation_errors += 1;
            }
        }
    }
    
    if validation_errors > 0 {
        println!("\n{} validation errors found", validation_errors);
        return Err(anyhow::anyhow!("Validation failed"));
    } else {
        println!("\n✓ All files are valid");
    }
    
    Ok(())
}

async fn show_instance_status(config_manager: &ConfigManager, instance_name: &str) -> Result<()> {
    // Check if instance exists
    if !std::path::Path::new(instance_name).exists() {
        println!("  Instance '{}' not found", instance_name);
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
                    println!("    - {}", file);
                }
            }
        }
        Err(_) => {
            println!("  Git: No repository (run gcgit xsiam pull to initialise)");
        }
    }
    
    // XSIAM connectivity status
    match config_manager.load_instance_config(instance_name) {
        Ok(instance_config) => {
            let xsiam_client = XsiamClient::new(instance_config);
            match xsiam_client.test_connectivity().await {
                Ok(_) => println!("  XSIAM: Connected"),
                Err(e) => println!("  XSIAM: Connection failed - {}", e),
            }
        }
        Err(_) => println!("  XSIAM: Configuration not found"),
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
            differences.push(format!("  → Name: '{}' → '{}'", 
                truncate_string(&local.name, 30), 
                truncate_string(&remote.name, 30)));
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
                println!("{}", diff);
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
