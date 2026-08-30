// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// CWP (Cloud Workload Protection) module implementation
// Supports 1 content type: policies (v2).
//
// Wire format (live-probed against a Cortex tenant):
//  - GET /public_api/v2/cwp/policies -> 200, JSON array at root (id is UUID).
//
// registry_onboarding was removed in 2.5.1. gcgit called
// /public_api/v1/cwp/registry_onboarding/instances as though it listed connectors.
// It does not: the documented route is
// /public_api/v1/cwp/registry_onboarding/instances/{connectorID} and returns one
// connector. Called without an ID the platform answered 403 with "Insufficient
// permissions for api key", which sent people to check entitlements for a problem
// that was neither about permissions nor about licensing.
//
// No companion endpoint lists connector IDs, so there is nothing to enumerate from
// and the content type could not be made to work. If a listing route appears it can
// come back, following the NameListed pattern used for RBAC roles.

use super::{ContentTypeDefinition, Module, PullStrategy};

pub struct CwpModule;

impl Module for CwpModule {
    fn id(&self) -> &'static str {
        "cwp"
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
                // Both are bumped server-side on every read: two consecutive pulls of
                // the same unmodified policy report values seconds apart. AppSec also
                // has a `policies` content type whose createdAt is genuine, which is
                // why this belongs on the definition rather than in a shared match.
                dedupe_by_latest: None,
                excluded_fields: &["createdAt", "modifiedAt"],
                set_valued_fields: &[],
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
        assert_eq!(module.base_api_path(), "/public_api");
    }

    #[test]
    fn test_cwp_content_types() {
        let module = CwpModule;
        let types = module.content_types();
        assert_eq!(types.len(), 1);
        let names: Vec<&str> = types.iter().map(|t| t.name).collect();
        assert!(names.contains(&"policies"));
        // registry_onboarding was removed: its endpoint fetches one connector by ID
        // and nothing lists the IDs, so it could never be enumerated.
        assert!(!names.contains(&"registry_onboarding"));
    }

    #[test]
    fn test_cwp_policies_endpoint() {
        let module = CwpModule;
        let types = module.content_types();
        let policies = types.iter().find(|t| t.name == "policies").unwrap();
        assert_eq!(policies.get_endpoint, "v2/cwp/policies");
        assert!(policies.request_body.is_none(), "policies should be GET");
    }
}
