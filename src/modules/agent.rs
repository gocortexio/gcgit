// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// Agent Configurations module implementation
// Supports 10 singleton content types under /public_api/v1/configurations/agent/*
//
// Wire format: although the published Cortex spec describes these endpoints as
// no-parameter GETs, live Cortex tenants return HTTP 500 for GET. POST with body
// {"request_data":{}} returns HTTP 200 with payload {"reply": {...singleton...}}
// on every one of the 10 endpoints.
//
// All content types therefore set `request_body = Some({"request_data":{}})`
// (which the puller treats as a POST request) and `response_path = Some("reply")`.
// `api.rs::extract_items_from_response` wraps the singleton object into a
// one-element vec when the resolved path yields an object instead of an array.

use super::{Module, ContentTypeDefinition, PullStrategy};
use serde_json::json;

pub struct AgentModule;

/// All Agent Configurations content types are global singletons.
/// Helper used by api.rs and types.rs to apply singleton response/id handling.
pub const AGENT_SINGLETONS: &[&str] = &[
    "content_management",
    "agent_status",
    "auto_upgrade",
    "wildfire_analysis",
    "informative_btp_issues",
    "cortex_xdr_log_collection",
    "action_center_expiration",
    "critical_environment_versions",
    "advanced_analysis",
    "endpoint_administration_cleanup",
];

pub fn is_agent_singleton(name: &str) -> bool {
    AGENT_SINGLETONS.contains(&name)
}

impl Module for AgentModule {
    fn id(&self) -> &'static str {
        "agent"
    }

    fn name(&self) -> &'static str {
        "Agent Configurations"
    }

    fn base_api_path(&self) -> &'static str {
        "/public_api/v1/configurations/agent"
    }

    fn content_types(&self) -> Vec<ContentTypeDefinition> {
        AGENT_SINGLETONS
            .iter()
            .map(|name| ContentTypeDefinition {
                name,
                get_endpoint: name,
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "name",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("reply"),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_module_metadata() {
        let module = AgentModule;
        assert_eq!(module.id(), "agent");
        assert_eq!(module.name(), "Agent Configurations");
        assert_eq!(module.base_api_path(), "/public_api/v1/configurations/agent");
    }

    #[test]
    fn test_agent_content_types() {
        let module = AgentModule;
        let types = module.content_types();
        assert_eq!(types.len(), 10);
        let names: Vec<&str> = types.iter().map(|t| t.name).collect();
        for expected in AGENT_SINGLETONS {
            assert!(names.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn test_is_agent_singleton() {
        assert!(is_agent_singleton("agent_status"));
        assert!(is_agent_singleton("endpoint_administration_cleanup"));
        assert!(!is_agent_singleton("dashboards"));
        assert!(!is_agent_singleton("application_configuration"));
    }
}
