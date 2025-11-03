// Module system for gcgit - trait-based plugin architecture
// Each Cortex module (XSIAM, AppSec, etc.) implements the Module trait

use serde_json::Value;
use std::collections::HashMap;

// Module implementations
mod xsiam;
mod appsec;

/// Core trait that all modules must implement
/// Note: Some methods may not be actively called but define the module contract
pub trait Module: Send + Sync {
    /// Unique module identifier (e.g., "xsiam", "appsec")
    /// Used in CLI commands and config.toml [modules.<id>]
    fn id(&self) -> &'static str;
    
    /// Human-readable module name (e.g., "XSIAM", "Application Security")
    /// Part of module contract - available for future UI/display features
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    
    /// Get all content types supported by this module
    fn content_types(&self) -> Vec<ContentTypeDefinition>;
    
    /// Base API path for this module (e.g., "/public_api/v1")
    fn base_api_path(&self) -> &'static str;
}

/// Module configuration from config.toml [modules.<name>] blocks
#[derive(Debug, Clone)]
pub struct ModuleConfig {
    pub enabled: bool,
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
}

/// Definition of a content type within a module
#[derive(Debug, Clone)]
pub struct ContentTypeDefinition {
    /// Name used in directories and CLI (e.g., "dashboards", "applications")
    pub name: &'static str,
    
    /// API endpoint for retrieving items (relative to base_api_path)
    pub get_endpoint: &'static str,
    
    /// Pull strategy to use for this content type
    pub pull_strategy: PullStrategy,
    
    /// Field name for unique ID in API responses
    pub id_field: &'static str,
    
    /// Optional: Request body for POST endpoints
    pub request_body: Option<Value>,
    
    /// Optional: Response path to extract items from JSON
    /// Examples: "reply", "objects[0].dashboards_data", "data"
    pub response_path: Option<&'static str>,
}

/// Pull strategy defines how to retrieve content from APIs
#[derive(Debug, Clone)]
pub enum PullStrategy {
    /// Standard JSON collection - single API call returns all items
    /// Used by: XSIAM correlations, biocs, dashboards, widgets
    JsonCollection,
    
    /// Paginated API - requires multiple requests with page/pageSize params
    /// Used by: AppSec applications, repositories, integrations
    Paginated {
        page_param: &'static str,
        page_size_param: &'static str,
        page_size: usize,
    },
    
    /// ZIP artifact - two-step process: list metadata, then download ZIPs
    /// Used by: Future content types that require ZIP file downloads
    #[allow(dead_code)]
    ZipArtifact {
        metadata_endpoint: &'static str,
        download_endpoint: &'static str,
        metadata_response_path: &'static str,
        download_filter_field: &'static str,
    },
    
    /// Script code retrieval - two-step process: list scripts, then fetch code by UID
    /// Used by: XSIAM scripts (list scripts + individual code retrieval via script_uid)
    ScriptCode {
        list_endpoint: &'static str,
        code_endpoint: &'static str,
        list_response_path: &'static str,
        uid_field: &'static str,
    },
}

/// Registry of all available modules
pub struct ModuleRegistry {
    modules: HashMap<&'static str, Box<dyn Module>>,
}

impl ModuleRegistry {
    /// Load all registered modules
    pub fn load() -> Self {
        let mut modules: HashMap<&'static str, Box<dyn Module>> = HashMap::new();
        
        // Register all modules here
        modules.insert("xsiam", Box::new(xsiam::XsiamModule));
        modules.insert("appsec", Box::new(appsec::AppSecModule));
        
        Self { modules }
    }
    
    /// Get a module by ID
    pub fn get(&self, id: &str) -> Option<&dyn Module> {
        self.modules.get(id).map(|m| m.as_ref())
    }
    
    /// Get all module IDs - useful for dynamic module discovery
    #[allow(dead_code)]
    pub fn module_ids(&self) -> Vec<&'static str> {
        self.modules.keys().copied().collect()
    }
    
    /// Get all modules
    pub fn all_modules(&self) -> Vec<&dyn Module> {
        self.modules.values().map(|m| m.as_ref()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_registry_loads_modules() {
        let registry = ModuleRegistry::load();
        
        // Should load both XSIAM and AppSec modules
        assert!(registry.get("xsiam").is_some());
        assert!(registry.get("appsec").is_some());
        
        // Module IDs should match
        let xsiam = registry.get("xsiam").unwrap();
        assert_eq!(xsiam.id(), "xsiam");
        
        let appsec = registry.get("appsec").unwrap();
        assert_eq!(appsec.id(), "appsec");
    }
    
    #[test]
    fn test_module_content_types() {
        let registry = ModuleRegistry::load();
        
        // XSIAM should have 6 content types
        let xsiam = registry.get("xsiam").unwrap();
        let xsiam_types = xsiam.content_types();
        assert_eq!(xsiam_types.len(), 6);
        
        // AppSec should have 5 content types
        let appsec = registry.get("appsec").unwrap();
        let appsec_types = appsec.content_types();
        assert_eq!(appsec_types.len(), 5);
    }
}
