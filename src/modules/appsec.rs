// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// AppSec module implementation
// Supports 7 content types: applications, policies, rules, repositories, integrations,
// application_configuration, application_criteria

use super::{Module, ContentTypeDefinition, PullStrategy};

pub struct AppSecModule;

impl Module for AppSecModule {
    fn id(&self) -> &'static str {
        "appsec"
    }
    
    fn name(&self) -> &'static str {
        "Application Security"
    }
    
    fn base_api_path(&self) -> &'static str {
        "/public_api"
    }
    
    fn content_types(&self) -> Vec<ContentTypeDefinition> {
        vec![
            // Applications - Paginated GET endpoint
            ContentTypeDefinition {
                name: "applications",
                get_endpoint: "appsec/v1/application",
                pull_strategy: PullStrategy::Paginated {
                    page_param: "page",
                    page_size_param: "pageSize",
                    page_size: 100,
                },
                id_field: "id",
                request_body: None,
                response_path: Some("data"),
            },
            
            // Policies - Security policies for threat detection (returns array at root)
            ContentTypeDefinition {
                name: "policies",
                get_endpoint: "appsec/v1/policies",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: None,
                response_path: None,
            },
            
            // Rules - Custom security rules (returns {"offset": X, "rules": [...]})
            ContentTypeDefinition {
                name: "rules",
                get_endpoint: "appsec/v1/rules",
                pull_strategy: PullStrategy::OffsetPaginated {
                    offset_param: "offset",
                    limit_param: "limit",
                    page_size: 100,
                },
                id_field: "id",
                request_body: None,
                response_path: Some("rules"),
            },
            
            // Repositories - Code repository configurations
            ContentTypeDefinition {
                name: "repositories",
                get_endpoint: "appsec/v1/repositories",
                pull_strategy: PullStrategy::OffsetPaginated {
                    offset_param: "offset",
                    limit_param: "limit",
                    page_size: 100,
                },
                id_field: "id",
                request_body: None,
                response_path: None,
            },
            
            // Integrations - External data source integrations (returns array at root)
            ContentTypeDefinition {
                name: "integrations",
                get_endpoint: "appsec/v1/integrations",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: None,
                response_path: None,
            },
            
            // Application configuration - Singleton configuration endpoint
            ContentTypeDefinition {
                name: "application_configuration",
                get_endpoint: "appsec/v1/application/configuration",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: None,
                response_path: None,
            },
            
            // Application criteria - Filtering criteria for applications
            ContentTypeDefinition {
                name: "application_criteria",
                get_endpoint: "appsec/v1/application/criteria/all",
                pull_strategy: PullStrategy::Paginated {
                    page_param: "page",
                    page_size_param: "pageSize",
                    page_size: 100,
                },
                id_field: "id",
                request_body: None,
                response_path: Some("items"),
            },
            
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_appsec_module_metadata() {
        let module = AppSecModule;
        
        assert_eq!(module.id(), "appsec");
        assert_eq!(module.name(), "Application Security");
        assert_eq!(module.base_api_path(), "/public_api");
    }
    
    #[test]
    fn test_appsec_content_types() {
        let module = AppSecModule;
        let types = module.content_types();
        
        // Should have 7 content types
        assert_eq!(types.len(), 7);
        
        // Check content type names
        let type_names: Vec<&str> = types.iter().map(|t| t.name).collect();
        assert!(type_names.contains(&"applications"));
        assert!(type_names.contains(&"policies"));
        assert!(type_names.contains(&"rules"));
        assert!(type_names.contains(&"repositories"));
        assert!(type_names.contains(&"integrations"));
        assert!(type_names.contains(&"application_configuration"));
        assert!(type_names.contains(&"application_criteria"));
    }
    
    #[test]
    fn test_applications_uses_pagination() {
        let module = AppSecModule;
        let types = module.content_types();
        
        let apps = types.iter().find(|t| t.name == "applications").unwrap();
        
        // Applications should use Paginated pull strategy
        match &apps.pull_strategy {
            PullStrategy::Paginated { page_size, .. } => {
                assert_eq!(*page_size, 100);
            },
            _ => panic!("Applications should use Paginated pull strategy"),
        }
    }
    
    #[test]
    fn test_repositories_uses_offset_paginated() {
        let module = AppSecModule;
        let types = module.content_types();
        
        let repos = types.iter().find(|t| t.name == "repositories").unwrap();
        match &repos.pull_strategy {
            PullStrategy::OffsetPaginated { page_size, .. } => {
                assert_eq!(*page_size, 100);
            },
            _ => panic!("Repositories should use OffsetPaginated pull strategy"),
        }
    }
    
    #[test]
    fn test_integrations_uses_json_collection() {
        let module = AppSecModule;
        let types = module.content_types();
        
        let integrations = types.iter().find(|t| t.name == "integrations").unwrap();
        assert!(matches!(integrations.pull_strategy, PullStrategy::JsonCollection));
    }
    
    #[test]
    fn test_all_get_endpoints_valid() {
        let module = AppSecModule;
        let types = module.content_types();
        
        // All endpoints should start with "appsec/v1/"
        for content_type in types {
            assert!(
                content_type.get_endpoint.starts_with("appsec/v1/"),
                "Endpoint {} should start with 'appsec/v1/'",
                content_type.get_endpoint
            );
        }
    }
}
