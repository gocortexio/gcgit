// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

// Module system for gcgit - trait-based plugin architecture
// Each Cortex module (XSIAM, AppSec, etc.) implements the Module trait

use serde_json::Value;
use std::collections::HashMap;

// Module implementations
pub mod agent;
mod appsec;
mod cwp;
mod xsiam;

/// Core trait that all modules must implement.
/// Some methods define the module contract and may not be actively called.
pub trait Module: Send + Sync {
    /// Unique module identifier (e.g., "platform", "appsec")
    /// Used in CLI commands and config.toml [modules.<id>]
    fn id(&self) -> &'static str;

    /// Identifier this module was previously known by, if it has been renamed.
    ///
    /// Existing instances keep their directory and their config.toml section under the
    /// old name. A rename that silently orphaned pulled files or stopped resolving
    /// credentials would be worse than the inconsistent name it fixes, so both are
    /// accepted and the old one continues to be used where it is already in place.
    fn legacy_id(&self) -> Option<&'static str> {
        None
    }

    /// Get all content types supported by this module
    fn content_types(&self) -> Vec<ContentTypeDefinition>;

    /// Base API path for this module (e.g., "/public_api/v1")
    fn base_api_path(&self) -> &'static str;

    /// Endpoint used to verify that the tenant is reachable and the credentials work.
    ///
    /// Defaults to the platform health check, which every Cortex tenant exposes and
    /// which returns 401, 402 or 403 distinctly. A leading `/` makes the path
    /// absolute, so it resolves the same way regardless of the module's base path.
    fn connectivity_endpoint(&self) -> &'static str {
        "/public_api/v1/healthcheck"
    }
}

/// Module configuration from config.toml [modules.<name>] blocks
#[derive(Debug, Clone)]
pub struct ModuleConfig {
    pub enabled: bool,
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
    /// Send requests over plain HTTP instead of HTTPS.
    ///
    /// Off unless the connection explicitly opts in. Intended for pointing gcgit at
    /// a local mock or a development endpoint that does not terminate TLS.
    pub force_http: bool,
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

    /// Field used to choose between records that share an identifier.
    ///
    /// An endpoint may return the same record more than once, for example once per
    /// category it belongs to. Where the copies differ, the one with the greatest
    /// value of this field wins, so the stored copy reflects the most recent state of
    /// the record rather than whichever copy happened to arrive first.
    ///
    /// Without it, duplicates are resolved by serialised form, which is deterministic
    /// but arbitrary.
    pub dedupe_by_latest: Option<&'static str>,

    /// Fields the platform changes without the configuration having changed.
    ///
    /// These are dropped before storage so a pull of an unmodified tenant produces no
    /// diff. Typically server-maintained timestamps and usage counters. Add a field
    /// here on evidence that it moves between pulls of an unchanged object: dropping
    /// one that carries real configuration would hide a change rather than a
    /// distraction.
    ///
    /// Declared per content type rather than matched centrally, so two modules that
    /// share a content type name each carry their own list.
    pub excluded_fields: &'static [&'static str],

    /// Fields whose array value is semantically a set rather than a sequence.
    ///
    /// The platform returns the members of these fields in an arbitrary order, so
    /// they are sorted before storage to stop every pull producing a diff. Only add
    /// a field here on evidence that its order carries no meaning: sorting a field
    /// whose order is significant would mean the stored YAML no longer describes the
    /// platform, which is the mistake a blanket sort of every string array made.
    pub set_valued_fields: &'static [&'static str],
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

    /// Script code retrieval - two-step process: list scripts, then fetch code by UID
    /// Used by: XSIAM scripts (list scripts + individual code retrieval via script_uid)
    ScriptCode {
        list_endpoint: &'static str,
        code_endpoint: &'static str,
        list_response_path: &'static str,
        uid_field: &'static str,
    },

    /// Offset-based pagination - uses offset and limit query parameters for sequential retrieval
    /// Used by: AppSec rules, repositories, scans (returns batches with an offset marker)
    OffsetPaginated {
        offset_param: &'static str,
        limit_param: &'static str,
        page_size: usize,
    },

    /// Window pagination expressed in the POST body rather than the query string.
    ///
    /// The endpoint takes an absolute half-open range and reports the size of the
    /// full collection, so iteration continues until that many items have been seen.
    /// Used by: attack surface rules, which cap a single response well below the
    /// total number of rules.
    BodyWindowPaginated {
        from_param: &'static str,
        to_param: &'static str,
        total_field: &'static str,
        page_size: usize,
    },

    /// Two-step retrieval for endpoints that cannot enumerate their own contents.
    ///
    /// The RBAC role and user-group endpoints reject a request that does not name at
    /// least one record, so there is no way to ask for all of them. The names are
    /// instead harvested from another endpoint that can be enumerated, and passed
    /// back in a single follow-up request.
    ///
    /// This means coverage is limited to records referenced by the source endpoint:
    /// a role assigned to no user cannot be discovered this way.
    NameListed {
        source_endpoint: &'static str,
        source_response_path: &'static str,
        source_name_field: &'static str,
        names_param: &'static str,
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
        modules.insert("platform", Box::new(xsiam::XsiamModule));
        modules.insert("appsec", Box::new(appsec::AppSecModule));
        modules.insert("agent", Box::new(agent::AgentModule));
        modules.insert("cwp", Box::new(cwp::CwpModule));

        Self { modules }
    }

    /// Get a module by its identifier, or by an identifier it was previously known by.
    pub fn get(&self, id: &str) -> Option<&dyn Module> {
        if let Some(module) = self.modules.get(id) {
            return Some(module.as_ref());
        }
        self.modules
            .values()
            .find(|module| module.legacy_id() == Some(id))
            .map(|m| m.as_ref())
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

        // Should load XSIAM, AppSec, Agent, and CWP modules
        assert!(registry.get("platform").is_some());
        // The previous name still resolves so existing scripts keep working.
        assert!(registry.get("xsiam").is_some());
        assert!(registry.get("appsec").is_some());
        assert!(registry.get("agent").is_some());
        assert!(registry.get("cwp").is_some());

        // Module IDs should match
        let platform = registry.get("platform").unwrap();
        assert_eq!(platform.id(), "platform");
        assert_eq!(platform.legacy_id(), Some("xsiam"));
        // Both names reach the same module.
        assert_eq!(registry.get("xsiam").unwrap().id(), "platform");

        let appsec = registry.get("appsec").unwrap();
        assert_eq!(appsec.id(), "appsec");
    }

    #[test]
    fn test_module_content_types() {
        let registry = ModuleRegistry::load();

        // Platform should have 14 content types
        let platform = registry.get("platform").unwrap();
        let platform_types = platform.content_types();
        assert_eq!(platform_types.len(), 14);

        // AppSec should have 7 content types
        let appsec = registry.get("appsec").unwrap();
        let appsec_types = appsec.content_types();
        assert_eq!(appsec_types.len(), 7);

        // Agent should have 10 content types
        let agent = registry.get("agent").unwrap();
        let agent_types = agent.content_types();
        assert_eq!(agent_types.len(), 10);

        // CWP should have 1 content type
        let cwp = registry.get("cwp").unwrap();
        let cwp_types = cwp.content_types();
        assert_eq!(cwp_types.len(), 1);
    }
}
