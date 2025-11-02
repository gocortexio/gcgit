use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use chrono::{DateTime, Utc};

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
    
    #[serde(flatten)]
    pub content: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObjectMetadata {
    pub created_by: String,
    pub version: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    
    #[serde(flatten)]
    pub additional: HashMap<String, Value>,
}

impl Default for ObjectMetadata {
    fn default() -> Self {
        Self {
            created_by: "gcgit".to_string(),
            version: "unknown".to_string(),
            created_at: None,
            updated_at: None,
            additional: HashMap::new(),
        }
    }
}

impl XsiamObject {
    #[allow(dead_code)]
    pub fn new(id: String, name: String, content_type: String) -> Self {
        Self {
            id,
            name: Some(name),
            description: String::new(),
            content_type,
            metadata: ObjectMetadata::default(),
            tenant_id: None,
            content: HashMap::new(),
        }
    }

    pub fn from_api_response(json: &Value, content_type: &str) -> Result<Self> {
        // Handle different ID field names based on content type
        let id = match content_type {
            "correlation_searches" | "biocs" => {
                // For correlation rules and BIOCs, use rule_id as the primary ID
                json.get("rule_id")
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
                    .or_else(|| json.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .unwrap_or_default()
            }
            "widgets" => {
                // Widgets use creation_time or global_id as unique identifier
                json.get("creation_time")
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
                    .or_else(|| json.get("global_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .or_else(|| json.get("widget_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .or_else(|| json.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("widget_{}", chrono::Utc::now().timestamp()))
            }
            "dashboards" => {
                // Dashboards use global_id or default_dashboard_id as unique identifier
                json.get("global_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| json.get("default_dashboard_id").and_then(|v| v.as_i64()).map(|i| i.to_string()))
                    .or_else(|| json.get("dashboard_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .or_else(|| json.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("dashboard_{}", chrono::Utc::now().timestamp()))
            }
            "authentication_settings" => {
                // Authentication settings use name field as unique identifier
                json.get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| json.get("setting_name").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .or_else(|| json.get("type").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("auth_setting_{}", chrono::Utc::now().timestamp()))
            }
            _ => {
                json.get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| json.get("id").and_then(|v| v.as_i64()).map(|i| i.to_string()))
                    .unwrap_or_else(|| format!("object_{}", chrono::Utc::now().timestamp()))
            }
        };

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
            _ => {
                json.get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
        };

        let description = json.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let mut metadata = ObjectMetadata::default();
        
        // Extract metadata if present, preserving original timestamps
        if let Some(meta) = json.get("metadata") {
            if let Ok(parsed_meta) = serde_json::from_value::<ObjectMetadata>(meta.clone()) {
                metadata = parsed_meta;
            }
        } else {
            // Extract timestamps from XSIAM API fields - try multiple common field names
            metadata.created_at = Self::extract_timestamp_from_json(json, &[
                "creation_time", "created_time", "created_at", "createdTime", 
                "date_created", "dateCreated"
            ]);
            
            metadata.updated_at = Self::extract_timestamp_from_json(json, &[
                "modification_time", "modified_time", "updated_at", "updatedTime",
                "last_modified", "lastModified", "date_modified", "dateModified"
            ]);
            
            // Extract version from XSIAM API - try multiple version fields
            metadata.version = json.get("version")
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
            json.get("tenant_id")
                .and_then(|v| {
                    if let Some(s) = v.as_str() {
                        Some(s.to_string())
                    } else { v.as_i64().map(|i| i.to_string()) }
                })
        } else {
            None
        };

        // Extract additional content, excluding tenant_id if it's for authentication_settings
        let mut content = HashMap::new();
        for (key, value) in json.as_object().unwrap_or(&serde_json::Map::new()) {
            let should_exclude = matches!(key.as_str(), "id" | "name" | "description" | "metadata") ||
                (content_type == "authentication_settings" && key == "tenant_id");
            
            if !should_exclude {
                content.insert(key.clone(), value.clone());
            }
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
                    let seconds = if timestamp > 10000000000 { // If > year 2001 in milliseconds
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

    #[allow(dead_code)]
    pub fn to_api_payload(&self) -> Value {
        let mut payload = serde_json::Map::new();
        
        payload.insert("id".to_string(), Value::String(self.id.clone()));
        if let Some(name) = &self.name {
            payload.insert("name".to_string(), Value::String(name.clone()));
        }
        payload.insert("description".to_string(), Value::String(self.description.clone()));
        
        // Add metadata
        if let Ok(metadata_value) = serde_json::to_value(&self.metadata) {
            payload.insert("metadata".to_string(), metadata_value);
        }
        
        // Add content fields
        for (key, value) in &self.content {
            payload.insert(key.clone(), value.clone());
        }

        Value::Object(payload)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub title: String,
    pub panel_type: String,
    pub query: String,
    pub visualization: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiocIndicator {
    pub indicator_type: String,
    pub pattern: String,
    pub description: String,
    pub threshold: Option<i32>,
    pub timeframe: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationSearchRule {
    pub rule_type: String,
    pub query: String,
    pub schedule: String,
    pub severity: String,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub language: String,
    pub source_code: String,
    pub parameters: Vec<ScriptParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptParameter {
    pub name: String,
    pub parameter_type: String,
    pub required: bool,
    pub default_value: Option<String>,
    pub description: Option<String>,
}
