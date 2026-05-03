// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// CWP (Cloud Workload Protection) module implementation
// Supports 2 content types: policies (v2), registry_onboarding (v1).
//
// Both endpoints are GET with all-optional query parameters, so they slot
// into JsonCollection. Versions are mixed (v1 + v2) within a single module,
// so we keep the module base path at `/public_api` and embed the version
// in each per-content-type `get_endpoint` (mirrors how AppSec does it).
//
// Wire format (live-probed against a Cortex tenant):
//  - GET /public_api/v2/cwp/policies -> 200, JSON array at root (id is UUID).
//  - GET /public_api/v1/cwp/registry_onboarding/instances -> may return 403
//    "Insufficient permissions" on tenants without the Cortex Cloud Runtime
//    Security add-on (e.g. XSIAM Enterprise Plus only). The pull/test loop
//    in main.rs already logs a per-endpoint warning and continues, so a 403
//    on this content type does not abort the whole module.

use super::{Module, ContentTypeDefinition, PullStrategy};

pub struct CwpModule;

impl Module for CwpModule {
    fn id(&self) -> &'static str {
        "cwp"
    }

    fn name(&self) -> &'static str {
        "Cloud Workload Protection"
    }

    fn base_api_path(&self) -> &'static str {
        "/public_api"
    }

    fn content_types(&self) -> Vec<ContentTypeDefinition> {
        vec![
            // Policies - v2 endpoint, returns JSON array at root.
            // No `types` filter so we get all policy types in a single call.
            ContentTypeDefinition {
                name: "policies",
                get_endpoint: "v2/cwp/policies",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: None,
                response_path: None,
            },

            // Registry onboarding - v1 endpoint, returns the XSIAM
            // `{"reply": ...}` envelope (also seen on the 403 error body).
            ContentTypeDefinition {
                name: "registry_onboarding",
                get_endpoint: "v1/cwp/registry_onboarding/instances",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: None,
                response_path: Some("reply"),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cwp_module_metadata() {
        let module = CwpModule;
        assert_eq!(module.id(), "cwp");
        assert_eq!(module.name(), "Cloud Workload Protection");
        assert_eq!(module.base_api_path(), "/public_api");
    }

    #[test]
    fn test_cwp_content_types() {
        let module = CwpModule;
        let types = module.content_types();
        assert_eq!(types.len(), 2);
        let names: Vec<&str> = types.iter().map(|t| t.name).collect();
        assert!(names.contains(&"policies"));
        assert!(names.contains(&"registry_onboarding"));
    }

    #[test]
    fn test_cwp_endpoints_mixed_versions() {
        let module = CwpModule;
        let types = module.content_types();
        let policies = types.iter().find(|t| t.name == "policies").unwrap();
        let reg = types.iter().find(|t| t.name == "registry_onboarding").unwrap();
        assert_eq!(policies.get_endpoint, "v2/cwp/policies");
        assert_eq!(reg.get_endpoint, "v1/cwp/registry_onboarding/instances");
        assert!(policies.request_body.is_none(), "policies should be GET");
        assert!(reg.request_body.is_none(), "registry_onboarding should be GET");
    }
}
