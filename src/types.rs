// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XsiamObject {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub description: String,
    pub content_type: String,
    pub metadata: ObjectMetadata,

    // Authentication settings specific field
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,

    // A sorted map, not a hash map. Anything that serialises an XsiamObject as a
    // whole - notably the tie-break used when collapsing records that share an
    // identifier - would otherwise get a different byte string on every run, because
    // a hash map iterates in an arbitrary order. The written file was already sorted
    // explicitly, but the comparison keys derived from the object were not.
    #[serde(flatten)]
    pub content: BTreeMap<String, Value>,
}

/// Default author recorded when the platform does not supply one.
fn default_created_by() -> String {
    "gcgit".to_string()
}

/// Default version recorded when the platform does not supply one.
fn default_version() -> String {
    "unknown".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObjectMetadata {
    // `created_by` and `version` carry defaults so that a platform-supplied
    // `metadata` object which does not contain them still deserialises rather
    // than being discarded.  Cortex dashboards, for example, return
    // `metadata: {"params": [...]}`; without these defaults that object failed
    // to parse and its contents were silently dropped.
    #[serde(default = "default_created_by")]
    pub created_by: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,

    // A sorted map, not a hash map: this is flattened straight into the serialised
    // metadata block without passing through the key sorting that content fields
    // get, so a hash map's arbitrary iteration order would rewrite the file on
    // every pull whenever the platform supplies two or more extra metadata keys.
    #[serde(flatten)]
    pub additional: BTreeMap<String, Value>,
}

impl Default for ObjectMetadata {
    fn default() -> Self {
        Self {
            created_by: default_created_by(),
            version: default_version(),
            created_at: None,
            updated_at: None,
            additional: BTreeMap::new(),
        }
    }
}

impl XsiamObject {
    /// Return the first usable identifier found among `fields`.
    ///
    /// Accepts either a JSON string or a JSON number, because Cortex is not
    /// consistent about which it uses for a given field. Correlation rules, for
    /// example, are documented as returning an integer `id` while BIOCs return an
    /// integer `rule_id`. Blank and whitespace-only strings count as absent.
    fn id_from_fields(json: &Value, fields: &[&str]) -> Option<String> {
        fields.iter().find_map(|field| {
            json.get(*field).and_then(|value| match value {
                Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
        })
    }

    /// Derive a stable identifier for an object that carries no usable ID field.
    ///
    /// The identifier is a 64-bit FNV-1a hash of the object's canonical JSON.
    /// serde_json stores object keys in a sorted map, so `to_string` produces the
    /// same bytes for the same payload on every run and the resulting identifier is
    /// reproducible. This matters because the identifier is written into the YAML
    /// and, for unnamed objects, into the filename: a clock-based fallback produced
    /// a different name on every pull, so repeated pulls accumulated duplicate files
    /// and every diff was noise.
    fn stable_fallback_id(prefix: &str, json: &Value) -> String {
        const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let canonical = serde_json::to_string(json).unwrap_or_default();
        let mut hash = FNV_OFFSET_BASIS;
        for byte in canonical.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }

        format!("{prefix}_{hash:016x}")
    }

    /// Build a stored object from one API response item.
    ///
    /// `excluded_fields` comes from the content type definition. It previously took a
    /// module identifier so that CWP and AppSec, which both have a `policies` content
    /// type, could be told apart for timestamp exclusion. Declaring the exclusions on
    /// the definition makes that distinction fall out naturally, so the identifier is
    /// no longer needed here.
    pub fn from_api_response(
        json: &Value,
        content_type: &str,
        excluded_fields: &[&str],
    ) -> Result<Self> {
        // Candidate ID fields per content type, in order of preference. Every
        // candidate accepts a string or a number; if none match, a deterministic
        // hash of the payload is used rather than a timestamp.
        let id = match content_type {
            // Correlation rules and BIOCs: BIOCs return `rule_id`; the 3.4 platform
            // specification documents correlation rules as returning an integer `id`.
            "correlation_searches" | "biocs" => {
                Self::id_from_fields(json, &["rule_id", "id", "global_id"])
            }
            "widgets" => Self::id_from_fields(
                json,
                &[
                    "widget_key",
                    "global_id",
                    "widget_id",
                    "id",
                    "creation_time",
                ],
            ),
            "dashboards" => Self::id_from_fields(
                json,
                &["global_id", "default_dashboard_id", "dashboard_id", "id"],
            ),
            "authentication_settings" => {
                Self::id_from_fields(json, &["name", "setting_name", "type"])
            }
            "scheduled_queries" => Self::id_from_fields(json, &["query_def_id"]),
            "xql_library" => Self::id_from_fields(json, &["id"]),
            "rbac_users" => Self::id_from_fields(json, &["user_email"]),
            // Roles carry no id; pretty_name is the key the platform accepts back.
            "rbac_roles" => Self::id_from_fields(json, &["pretty_name", "role_name", "name"]),
            // group_name is the unique key; pretty_name is only the display label.
            "rbac_user_groups" => Self::id_from_fields(json, &["group_name", "pretty_name"]),
            "attack_surface_rules" => Self::id_from_fields(
                json,
                &["attack_surface_rule_id", "attack_surface_rule_name"],
            ),
            // XQL get_datasets returns TitleCase keys with spaces ("Dataset Name") on
            // live tenants, while the published specification documents snake_case
            // (`dataset_name`). Accept either.
            "datasets" => Self::id_from_fields(json, &["Dataset Name", "dataset_name"]),
            // Singleton: the endpoint returns one configuration object with no ID.
            // "settings" matches the agent singletons and the documented layout, so
            // the file is written as application_configuration/settings.yaml rather
            // than repeating the content type name inside its own directory.
            "application_configuration" => {
                Self::id_from_fields(json, &["id"]).or_else(|| Some("settings".to_string()))
            }
            // Agent Configurations singletons - stable id "settings" so the file
            // is always written as agent/<content_type>/settings.yaml regardless
            // of the response payload (which has neither id nor name fields).
            t if crate::modules::agent::is_agent_singleton(t) => Some("settings".to_string()),
            "application_criteria" => Self::id_from_fields(json, &["id"]),
            _ => Self::id_from_fields(json, &["id"]),
        }
        .unwrap_or_else(|| Self::stable_fallback_id(content_type, json));

        let name = match content_type {
            "widgets" => {
                // For widgets, use 'title' field as specified: widgets_data.0.title
                json.get("title")
                    .and_then(|v| v.as_str())
                    .or_else(|| json.get("name").and_then(|v| v.as_str()))
                    .or_else(|| json.get("widget_name").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            }
            "dashboards" => {
                // For dashboards, use 'name' field as specified: dashboards_data.0.name
                json.get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
            "authentication_settings" => {
                // For authentication settings, use 'name' or 'setting_name' field
                json.get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| json.get("setting_name").and_then(|v| v.as_str()))
                    .or_else(|| json.get("type").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            }
            "rbac_users" => json
                .get("user_email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            "rbac_roles" | "rbac_user_groups" => {
                // pretty_name is the human label shown in the console and makes a
                // far more readable filename than the fully qualified group name.
                json.get("pretty_name")
                    .or_else(|| json.get("role_name"))
                    .or_else(|| json.get("group_name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
            "attack_surface_rules" => json
                .get("attack_surface_rule_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            "datasets" => json
                .get("Dataset Name")
                .or_else(|| json.get("dataset_name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            // Singletons: deterministic name so the file is "settings.yaml"
            "application_configuration" => Some("settings".to_string()),
            t if crate::modules::agent::is_agent_singleton(t) => Some("settings".to_string()),
            _ => json
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };

        let description = json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let mut metadata = ObjectMetadata::default();

        // A platform `metadata` value that cannot be interpreted as gcgit metadata
        // (an array or a scalar, for example). It is preserved in `content` under
        // `platform_metadata` so that nothing is lost. The key is deliberately
        // different from `metadata`: `XsiamObject` has a named `metadata` field, so
        // a `metadata` key inside `content` would collide with it on both write and
        // read-back.
        let mut unparsed_platform_metadata: Option<Value> = None;

        // True when the metadata block did not supply values and they should be
        // derived from the object's own top-level fields instead.
        let mut derive_metadata_from_top_level = true;

        // Extract metadata if present, preserving original timestamps
        if let Some(meta) = json.get("metadata") {
            match serde_json::from_value::<ObjectMetadata>(meta.clone()) {
                Ok(parsed_meta) => {
                    metadata = parsed_meta;
                    derive_metadata_from_top_level = false;
                }
                Err(_) => unparsed_platform_metadata = Some(meta.clone()),
            }
        }

        if derive_metadata_from_top_level {
            // Extract timestamps from XSIAM API fields - try multiple common field names
            metadata.created_at = Self::extract_timestamp_from_json(
                json,
                &[
                    "creation_time",
                    "created_time",
                    "created_at",
                    "createdTime",
                    "date_created",
                    "dateCreated",
                ],
            );

            metadata.updated_at = Self::extract_timestamp_from_json(
                json,
                &[
                    "modification_time",
                    "modified_time",
                    "updated_at",
                    "updatedTime",
                    "last_modified",
                    "lastModified",
                    "date_modified",
                    "dateModified",
                    "observationTime",
                    "lastTriggered",
                ],
            );

            // Extract version from XSIAM API - try multiple version fields
            metadata.version = json
                .get("version")
                .or_else(|| json.get("rule_version"))
                .or_else(|| json.get("object_version"))
                .or_else(|| json.get("schema_version"))
                .and_then(|v| v.as_str())
                .unwrap_or("1.0")
                .to_string();

            // Keep gcgit as created_by for version control tracking
            metadata.created_by = "gcgit".to_string();
        }

        // Extract tenant_id for authentication_settings
        let tenant_id = if content_type == "authentication_settings" {
            json.get("tenant_id").and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else {
                    v.as_i64().map(|i| i.to_string())
                }
            })
        } else {
            None
        };

        // Extract additional content, excluding tenant_id if it's for authentication_settings
        let mut content = BTreeMap::new();
        for (key, value) in json.as_object().unwrap_or(&serde_json::Map::new()) {
            // Structural fields, lifted to the top level of the stored object.
            let should_exclude = matches!(key.as_str(), "id" | "name" | "description" | "metadata")
                || (content_type == "authentication_settings" && key == "tenant_id")
                // Fields the content type declares as platform-maintained. Declared
                // per content type so two modules sharing a name each carry their own.
                || excluded_fields.contains(&key.as_str());

            if !should_exclude {
                content.insert(key.clone(), value.clone());
            }
        }

        // Retain a platform `metadata` value that could not be parsed, so that
        // configuration carried in it survives the round trip.
        if let Some(raw_metadata) = unparsed_platform_metadata {
            content.insert("platform_metadata".to_string(), raw_metadata);
        }

        Ok(Self {
            id,
            name,
            description,
            content_type: content_type.to_string(),
            metadata,
            tenant_id,
            content,
        })
    }

    // Helper method to extract timestamps from JSON with multiple field name attempts
    fn extract_timestamp_from_json(json: &Value, field_names: &[&str]) -> Option<DateTime<Utc>> {
        for field_name in field_names {
            if let Some(timestamp_value) = json.get(field_name) {
                // Try parsing as integer timestamp (milliseconds)
                if let Some(timestamp) = timestamp_value.as_i64() {
                    // Handle both seconds and milliseconds timestamps
                    let seconds = if timestamp > 10000000000 {
                        // If > year 2001 in milliseconds
                        timestamp / 1000
                    } else {
                        timestamp
                    };

                    if let Some(dt) = DateTime::from_timestamp(seconds, 0) {
                        return Some(dt);
                    }
                }

                // Try parsing as string timestamp
                if let Some(timestamp_str) = timestamp_value.as_str() {
                    // Try multiple timestamp formats
                    if let Ok(dt) = timestamp_str.parse::<DateTime<Utc>>() {
                        return Some(dt);
                    }
                    // Try ISO format with different patterns
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp_str) {
                        return Some(dt.with_timezone(&Utc));
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dashboard_metadata_params_are_preserved() {
        // Shape taken from the 3.4 platform specification: dashboards_data items
        // carry a `metadata` object of {"params": [...]}. It previously failed to
        // deserialise into ObjectMetadata and was silently discarded.
        let payload = json!({
            "global_id": "dash-1",
            "name": "Executive Overview",
            "status": "ENABLED",
            "metadata": {"params": [{"name": "region", "value": "emea"}]}
        });

        let object = XsiamObject::from_api_response(&payload, "dashboards", &[]).unwrap();

        assert_eq!(object.id, "dash-1");
        let params = object
            .metadata
            .additional
            .get("params")
            .expect("dashboard metadata params must be retained");
        assert_eq!(params, &json!([{"name": "region", "value": "emea"}]));
    }

    #[test]
    fn unparseable_platform_metadata_is_retained_in_content() {
        // A metadata value that is not an object cannot become ObjectMetadata, so it
        // is kept under a separate key rather than dropped.
        let payload = json!({"id": "x-1", "metadata": ["a", "b"]});

        let object = XsiamObject::from_api_response(&payload, "widgets", &[]).unwrap();

        assert_eq!(
            object.content.get("platform_metadata"),
            Some(&json!(["a", "b"]))
        );
    }

    #[test]
    fn fallback_id_is_stable_across_calls() {
        // An object with no usable identifier field must produce the same id every
        // time, otherwise each pull writes a new file and every diff is noise.
        let payload = json!({"title": "no identifier here", "value": 42});

        let first = XsiamObject::from_api_response(&payload, "widgets", &[]).unwrap();
        let second = XsiamObject::from_api_response(&payload, "widgets", &[]).unwrap();

        assert_eq!(first.id, second.id);
        assert!(
            first.id.starts_with("widgets_"),
            "unexpected id: {}",
            first.id
        );
        // Key order must not affect the derived identifier.
        let reordered = json!({"value": 42, "title": "no identifier here"});
        let third = XsiamObject::from_api_response(&reordered, "widgets", &[]).unwrap();
        assert_eq!(first.id, third.id);
    }

    #[test]
    fn fallback_id_differs_for_different_payloads() {
        let a = XsiamObject::from_api_response(&json!({"title": "one"}), "widgets", &[]).unwrap();
        let b = XsiamObject::from_api_response(&json!({"title": "two"}), "widgets", &[]).unwrap();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn correlation_rule_with_integer_id_resolves() {
        // The 3.4 specification documents correlation rules as returning an integer
        // `id` and no `rule_id`. This previously produced an empty id, which made
        // every correlation file fail validation on diff and validate.
        let payload = json!({
            "id": 4815,
            "name": "Suspicious PowerShell",
            "severity": "SEV_040_HIGH"
        });

        let object = XsiamObject::from_api_response(&payload, "correlation_searches", &[]).unwrap();

        assert_eq!(object.id, "4815");
    }

    #[test]
    fn bioc_rule_id_still_takes_precedence() {
        let payload = json!({"rule_id": 99, "id": 1, "name": "Test BIOC"});
        let object = XsiamObject::from_api_response(&payload, "biocs", &[]).unwrap();
        assert_eq!(object.id, "99");
    }

    #[test]
    fn blank_identifier_falls_back_rather_than_producing_empty_id() {
        let payload = json!({"id": "   ", "name": "Blank id"});
        let object = XsiamObject::from_api_response(&payload, "policies", &[]).unwrap();
        assert!(!object.id.is_empty(), "id must never be empty");
    }

    #[test]
    fn agent_singletons_use_a_stable_settings_id() {
        let object =
            XsiamObject::from_api_response(&json!({"enabled": true}), "agent_status", &[]).unwrap();
        assert_eq!(object.id, "settings");
        assert_eq!(object.name.as_deref(), Some("settings"));
    }

    #[test]
    fn each_module_carries_its_own_exclusions_for_a_shared_content_type_name() {
        // CWP and AppSec both have a content type called "policies". CWP bumps
        // createdAt and modifiedAt on every read; the AppSec ones are genuine. The
        // lists come from the real module definitions, so this fails if either module
        // is changed to declare the wrong thing.
        use crate::modules::ModuleRegistry;
        let registry = ModuleRegistry::load();

        let cwp_def = registry.get("cwp").unwrap().content_types();
        let cwp_policies = cwp_def.iter().find(|c| c.name == "policies").unwrap();
        let appsec_def = registry.get("appsec").unwrap().content_types();
        let appsec_policies = appsec_def.iter().find(|c| c.name == "policies").unwrap();

        let policy = json!({
            "id": "p-1",
            "name": "Runtime policy",
            "createdAt": "2026-01-01T00:00:00Z",
            "modifiedAt": "2026-01-02T00:00:00Z"
        });

        let cwp = XsiamObject::from_api_response(&policy, "policies", cwp_policies.excluded_fields)
            .unwrap();
        assert!(
            !cwp.content.contains_key("createdAt"),
            "CWP createdAt must be excluded"
        );
        assert!(
            !cwp.content.contains_key("modifiedAt"),
            "CWP modifiedAt must be excluded"
        );

        let appsec =
            XsiamObject::from_api_response(&policy, "policies", appsec_policies.excluded_fields)
                .unwrap();
        assert!(
            appsec.content.contains_key("createdAt"),
            "AppSec createdAt must be retained"
        );
    }

    #[test]
    fn attack_surface_rules_drop_the_regenerated_created_timestamp() {
        // A live tenant reported a different `created` for 685 unchanged rules half an
        // hour apart, so every pull rewrote them. `modified` was stable and is kept.
        use crate::modules::ModuleRegistry;
        let registry = ModuleRegistry::load();
        let types = registry.get("platform").unwrap().content_types();
        let def = types
            .iter()
            .find(|c| c.name == "attack_surface_rules")
            .unwrap();

        let rule = json!({
            "attack_surface_rule_id": "OpenLDAP",
            "attack_surface_rule_name": "OpenLDAP",
            "created": 1788003789000i64,
            "modified": 1700000000000i64,
            "enabled_status": "On"
        });
        let object =
            XsiamObject::from_api_response(&rule, "attack_surface_rules", def.excluded_fields)
                .unwrap();

        assert!(
            !object.content.contains_key("created"),
            "created is regenerated and must be excluded"
        );
        assert!(
            object.content.contains_key("modified"),
            "modified is stable and must be kept"
        );
        assert!(
            object.content.contains_key("enabled_status"),
            "configuration must survive"
        );
    }

    #[test]
    fn widget_prefers_the_stable_key_over_the_creation_timestamp() {
        let payload = json!({
            "widget_key": "xql_1646052681403",
            "title": "Widget A",
            "creation_time": 1653303166334i64
        });
        let object = XsiamObject::from_api_response(&payload, "widgets", &[]).unwrap();
        assert_eq!(object.id, "xql_1646052681403");
    }

    #[test]
    fn dataset_accepts_either_key_casing() {
        let title_case = XsiamObject::from_api_response(
            &json!({"Dataset Name": "xdr_data", "Type": "System"}),
            "datasets",
            &[],
        )
        .unwrap();
        let snake_case = XsiamObject::from_api_response(
            &json!({"dataset_name": "xdr_data", "type": "System"}),
            "datasets",
            &[],
        )
        .unwrap();
        assert_eq!(title_case.id, "xdr_data");
        assert_eq!(snake_case.id, "xdr_data");
    }
}
