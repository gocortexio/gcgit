// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// XSIAM module implementation
// Supports 13 content types: scripts, dashboards, biocs, correlation_searches, widgets,
// authentication_settings, scheduled_queries, xql_library, rbac_users, rbac_roles,
// rbac_user_groups, attack_surface_rules, datasets

use super::{ContentTypeDefinition, Module, PullStrategy};
use serde_json::json;

pub struct XsiamModule;

impl Module for XsiamModule {
    fn id(&self) -> &'static str {
        "platform"
    }

    /// Instances created before the rename keep using "xsiam".
    fn legacy_id(&self) -> Option<&'static str> {
        Some("xsiam")
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
                // One wrapper object per dashboard, not one wrapper holding them all, so
                // every element has to be gathered. Reading objects[0] retrieved a
                // single dashboard and discarded the rest.
                response_path: Some("objects[*].dashboards_data"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // BIOCs (Behavioural Indicators of Compromise) - Simple JSON collection
            ContentTypeDefinition {
                name: "biocs",
                get_endpoint: "bioc/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "rule_id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("objects"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // Correlation searches - Security correlation rules
            ContentTypeDefinition {
                name: "correlation_searches",
                get_endpoint: "correlations/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "rule_id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("objects"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // Widgets - Dashboard widgets.
            // `widget_key` is the stable identifier; `creation_time` was previously
            // used and is a timestamp, so two widgets created in the same
            // millisecond collided.
            ContentTypeDefinition {
                name: "widgets",
                get_endpoint: "widgets/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "widget_key",
                request_body: Some(json!({"request_data": {}})),
                // As dashboards: one wrapper object per widget.
                response_path: Some("objects[*].widgets_data"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // Authentication settings - SSO and authentication configurations
            ContentTypeDefinition {
                name: "authentication_settings",
                get_endpoint: "authentication-settings/get/settings",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "name",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("reply"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // Scripts - Two-step code retrieval via script_uid
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
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // Scheduled queries - XQL scheduled queries
            ContentTypeDefinition {
                name: "scheduled_queries",
                get_endpoint: "scheduled_queries/list",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "query_def_id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("reply.DATA"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &[],
            },
            // XQL Library - Reusable XQL query library.
            // This endpoint sits at /public_api/xql_library/get, outside the /v1
            // prefix the rest of the module uses, so the path is given in absolute
            // form (leading slash) rather than as a relative segment.
            ContentTypeDefinition {
                name: "xql_library",
                get_endpoint: "/public_api/xql_library/get",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: Some(json!({"request_data": {"extended_view": true}})),
                response_path: Some("reply.xql_queries"),
                // RELATIONS is the sharing permission set for a saved query, for
                // example ["EDIT", "SHARE", "VIEW", "PUBLISH", "OWNER"]. A live
                // tenant returns the same members in a different order on every
                // call, so without normalisation each pull rewrites the file.
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &["RELATIONS"],
            },
            // RBAC users - Role-based access control users
            ContentTypeDefinition {
                name: "rbac_users",
                get_endpoint: "rbac/get_users",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "user_email",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("reply"),
                // Changes whenever the user signs in, which is activity rather than
                // configuration.
                dedupe_by_latest: None,
                excluded_fields: &["last_logged_in"],
                set_valued_fields: &[],
            },
            // RBAC roles - permission definitions behind each role.
            //
            // rbac/get_roles rejects an empty or absent role_names list ("must
            // provide at least one role name"), so the names are harvested from
            // rbac/get_users first. Roles assigned to no user are therefore not
            // retrieved; there is no enumeration endpoint that would allow it.
            //
            // The response nests one array per requested name inside reply, so the
            // extraction flattens a single level.
            ContentTypeDefinition {
                name: "rbac_roles",
                get_endpoint: "rbac/get_roles",
                pull_strategy: PullStrategy::NameListed {
                    source_endpoint: "rbac/get_users",
                    source_response_path: "reply",
                    source_name_field: "role_name",
                    names_param: "role_names",
                },
                id_field: "pretty_name",
                request_body: None,
                response_path: Some("reply"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &["permissions", "groups", "users"],
            },
            // RBAC user groups - group membership and source.
            // Same two-step retrieval as roles: group_names is mandatory.
            ContentTypeDefinition {
                name: "rbac_user_groups",
                get_endpoint: "rbac/get_user_group",
                pull_strategy: PullStrategy::NameListed {
                    source_endpoint: "rbac/get_users",
                    source_response_path: "reply",
                    source_name_field: "groups",
                    names_param: "group_names",
                },
                id_field: "group_name",
                request_body: None,
                response_path: Some("reply"),
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &["user_email"],
            },
            // Attack surface rules - ASM detection rules.
            // A single response is capped below the total rule count, and the
            // endpoint reports total_count, so retrieval walks an absolute window.
            ContentTypeDefinition {
                name: "attack_surface_rules",
                get_endpoint: "get_attack_surface_rules",
                pull_strategy: PullStrategy::BodyWindowPaginated {
                    from_param: "search_from",
                    to_param: "search_to",
                    total_field: "total_count",
                    page_size: 200,
                },
                id_field: "attack_surface_rule_id",
                request_body: None,
                response_path: Some("reply.attack_surface_rules"),
                // `created` is regenerated server-side rather than recording when the
                // rule was made: two pulls half an hour apart reported different
                // values for 685 unchanged rules. `modified` was stable across the
                // same window and is kept.
                // The same rule is returned once per category it belongs to. Where
                // the copies differ, `modified` is what indicates the rule actually
                // changed, so the most recently modified copy is the one to store.
                dedupe_by_latest: Some("modified"),
                excluded_fields: &["created"],
                set_valued_fields: &["asm_alert_categories"],
            },
            // Installed content packs.
            //
            // This endpoint sits outside the /public_api namespace entirely, so the
            // path is absolute. It is a plain GET returning a root-level array; a live
            // tenant returned 95 packs with identical bytes across repeated calls.
            ContentTypeDefinition {
                name: "content_packs",
                get_endpoint: "/xsoar/contentpacks/metadata/installed",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "id",
                request_body: None,
                response_path: None,
                // propagationLabels is a label set rather than an ordered sequence.
                dedupe_by_latest: None,
                excluded_fields: &[],
                set_valued_fields: &["propagationLabels"],
            },
            // Datasets - XQL dataset definitions (System, Lookup, Raw, User, Snapshot, Correlation)
            // Endpoint: /public_api/v1/xql/get_datasets returns {"reply": [{...}]} with TitleCase
            // field names ("Dataset Name", "Type", etc). Runtime/usage fields are excluded in
            // types.rs::from_api_response to keep Git diffs stable.
            ContentTypeDefinition {
                name: "datasets",
                get_endpoint: "xql/get_datasets",
                pull_strategy: PullStrategy::JsonCollection,
                id_field: "Dataset Name",
                request_body: Some(json!({"request_data": {}})),
                response_path: Some("reply"),
                // Runtime and usage figures rather than configuration. Both the
                // TitleCase spelling live tenants return and the snake_case one the
                // specification documents are listed, so either shape is handled.
                dedupe_by_latest: None,
                excluded_fields: &[
                    "Last Updated",
                    "Total Size Stored",
                    "Average Daily Size",
                    "Total Events",
                    "Average Event Size",
                    "Hot Range",
                    "Cold Range",
                    "Total Days Stored",
                    "last_updated",
                    "total_size_stored",
                    "average_daily_size",
                    "total_events",
                    "average_event_size",
                    "hot_range",
                    "cold_range",
                    "total_days_stored",
                ],
                set_valued_fields: &[],
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

        assert_eq!(module.id(), "platform");
        assert_eq!(module.legacy_id(), Some("xsiam"));
        assert_eq!(module.base_api_path(), "/public_api/v1");
    }

    #[test]
    fn test_xsiam_content_types() {
        let module = XsiamModule;
        let types = module.content_types();

        // Should have 14 content types
        assert_eq!(types.len(), 14);

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
        assert!(type_names.contains(&"datasets"));
        assert!(type_names.contains(&"rbac_roles"));
        assert!(type_names.contains(&"rbac_user_groups"));
        assert!(type_names.contains(&"attack_surface_rules"));
        assert!(type_names.contains(&"content_packs"));
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
