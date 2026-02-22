// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// XSIAM module implementation
// Supports 9 content types: scripts, dashboards, biocs, correlation_searches, widgets,
// authentication_settings, scheduled_queries, xql_library, rbac_users

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
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("objects"),
            },
            
            // Correlation searches - Security correlation rules
            ContentTypeDefinition {
                name: "correlation_searches",
                get_endpoint: "correlations/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "rule_id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
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
            
            // Scripts - Two-step code retrieval via script_uid
            // Step 1: List scripts via scripts/get_scripts to get script_uid values
            // Step 2: Fetch code for each script via scripts/get_script_code with script_uid
            ContentTypeDefinition {
                name: "scripts",
                get_endpoint: "scripts/get_scripts",
                pull_strategy: PullStrategy::ScriptCode {
                    list_endpoint: "scripts/get_scripts",
                    code_endpoint: "scripts/get_script_code",
                    list_response_path: "reply.scripts",
                    uid_field: "script_uid",
                },
                id_field: "script_uid",
                request_body: Some(json!({"request_data": {}})),
                response_path: None,
            },
            
            // Scheduled queries - XQL scheduled queries
            ContentTypeDefinition {
                name: "scheduled_queries",
                get_endpoint: "scheduled_queries/list",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "query_def_id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("reply.DATA"),
            },
            
            // XQL Library - Reusable XQL query library
            // Note: This endpoint sits at /public_api/xql_library/get (no /v1/)
            ContentTypeDefinition {
                name: "xql_library",
                get_endpoint: "../xql_library/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("reply.xql_queries"),
            },
            
            
            // RBAC users - Role-based access control users
            ContentTypeDefinition {
                name: "rbac_users",
                get_endpoint: "rbac/get_users",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "user_email",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("reply"),
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
        
        // Should have 11 content types
        assert_eq!(types.len(), 9);
        
        // Check content type names
        let type_names: Vec<&str> = types.iter().map(|t| t.name).collect();
        assert!(type_names.contains(&"dashboards"));
        assert!(type_names.contains(&"biocs"));
        assert!(type_names.contains(&"correlation_searches"));
        assert!(type_names.contains(&"widgets"));
        assert!(type_names.contains(&"authentication_settings"));
        assert!(type_names.contains(&"scripts"));
        assert!(type_names.contains(&"scheduled_queries"));
        assert!(type_names.contains(&"xql_library"));
        assert!(type_names.contains(&"rbac_users"));
    }
    
    #[test]
    fn test_scripts_uses_script_code_strategy() {
        let module = XsiamModule;
        let types = module.content_types();
        
        let scripts = types.iter().find(|t| t.name == "scripts").unwrap();
        
        // Scripts should use ScriptCode pull strategy
        match &scripts.pull_strategy {
            PullStrategy::ScriptCode { .. } => (),
            _ => panic!("Scripts should use ScriptCode pull strategy"),
        }
    }
}
