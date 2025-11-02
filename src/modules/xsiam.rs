// XSIAM module implementation
// Supports 6 content types: scripts, dashboards, biocs, correlation_searches, widgets, authentication_settings

use super::{Module, ContentTypeDefinition, PullStrategy};
use serde_json::json;

pub struct XsiamModule;

impl Module for XsiamModule {
    fn id(&self) -> &'static str {
        "xsiam"
    }
    
    fn name(&self) -> &'static str {
        "XSIAM"
    }
    
    fn base_api_path(&self) -> &'static str {
        "/public_api/v1"
    }
    
    fn content_types(&self) -> Vec<ContentTypeDefinition> {
        vec![
            // Dashboards - JSON collection with nested response path
            ContentTypeDefinition {
                name: "dashboards",
                get_endpoint: "dashboards/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "global_id",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("objects[0].dashboards_data"),
            },
            
            // BIOCs (Behavioural Indicators of Compromise) - Simple JSON collection
            ContentTypeDefinition {
                name: "biocs",
                get_endpoint: "bioc/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "rule_id",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("objects"),
            },
            
            // Correlation searches - Security correlation rules
            ContentTypeDefinition {
                name: "correlation_searches",
                get_endpoint: "correlations/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "rule_id",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("objects"),
            },
            
            // Widgets - Dashboard widgets
            ContentTypeDefinition {
                name: "widgets",
                get_endpoint: "widgets/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "creation_time",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("objects[0].widgets_data"),
            },
            
            // Authentication settings - SSO and authentication configurations
            ContentTypeDefinition {
                name: "authentication_settings",
                get_endpoint: "authentication-settings/get/settings",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "name",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("reply"),
            },
            
            // Scripts - ZIP artifacts requiring two-step retrieval
            // Step 1: List metadata via scripts/get_scripts
            // Step 2: Download individual ZIP files via scripts/get
            ContentTypeDefinition {
                name: "scripts",
                get_endpoint: "scripts/get_scripts",
                pull_strategy: PullStrategy::ZipArtifact {
                    metadata_endpoint: "scripts/get_scripts",
                    download_endpoint: "scripts/get",
                    metadata_response_path: "reply.scripts",
                    download_filter_field: "name",
                },
                id_field: "script_id",
                request_body: Some(json!({"request_data": {}})),
                response_path: None,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_xsiam_module_metadata() {
        let module = XsiamModule;
        
        assert_eq!(module.id(), "xsiam");
        assert_eq!(module.name(), "XSIAM");
        assert_eq!(module.base_api_path(), "/public_api/v1");
    }
    
    #[test]
    fn test_xsiam_content_types() {
        let module = XsiamModule;
        let types = module.content_types();
        
        // Should have 6 content types
        assert_eq!(types.len(), 6);
        
        // Check content type names
        let type_names: Vec<&str> = types.iter().map(|t| t.name).collect();
        assert!(type_names.contains(&"dashboards"));
        assert!(type_names.contains(&"biocs"));
        assert!(type_names.contains(&"correlation_searches"));
        assert!(type_names.contains(&"widgets"));
        assert!(type_names.contains(&"authentication_settings"));
        assert!(type_names.contains(&"scripts"));
    }
    
    #[test]
    fn test_scripts_uses_zip_strategy() {
        let module = XsiamModule;
        let types = module.content_types();
        
        let scripts = types.iter().find(|t| t.name == "scripts").unwrap();
        
        // Scripts should use ZipArtifact pull strategy
        match &scripts.pull_strategy {
            PullStrategy::ZipArtifact { .. } => (),
            _ => panic!("Scripts should use ZipArtifact pull strategy"),
        }
    }
}
