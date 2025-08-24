use serde_json::Value;
use std::collections::HashMap;

/// Content type configuration for XSIAM API endpoints
#[derive(Debug, Clone)]
pub struct ContentTypeConfig {
    #[allow(dead_code)]
    pub name: &'static str,
    pub get_endpoint: &'static str,
    pub insert_endpoint: &'static str,
    pub delete_endpoint: &'static str,
    #[allow(dead_code)]
    pub id_field: &'static str,
    pub request_id_key: &'static str,
}

impl ContentTypeConfig {
    /// Get the request data structure for individual object lookup
    pub fn get_request_data(&self, id: &str) -> Value {
        match self.request_id_key {
            "rule_id" => {
                serde_json::json!({
                    "request_data": {
                        self.request_id_key: id.parse::<i32>().unwrap_or(0)
                    }
                })
            }
            _ => {
                serde_json::json!({
                    "request_data": {
                        self.request_id_key: id
                    }
                })
            }
        }
    }
}

/// Registry of all supported content types
pub struct ContentTypeRegistry {
    types: HashMap<&'static str, ContentTypeConfig>,
}

impl ContentTypeRegistry {
    pub fn new() -> Self {
        let mut types = HashMap::new();
        
        // Define all supported content types
        types.insert("dashboards", ContentTypeConfig {
            name: "dashboards",
            get_endpoint: "dashboards/get",
            insert_endpoint: "dashboards/insert",
            delete_endpoint: "dashboards/delete",
            id_field: "id",
            request_id_key: "dashboard_id",
        });
        
        types.insert("biocs", ContentTypeConfig {
            name: "biocs",
            get_endpoint: "bioc/get",
            insert_endpoint: "bioc/insert",
            delete_endpoint: "bioc/delete",
            id_field: "rule_id",
            request_id_key: "rule_id",
        });
        
        types.insert("correlation_searches", ContentTypeConfig {
            name: "correlation_searches",
            get_endpoint: "correlations/get",
            insert_endpoint: "correlations/insert",
            delete_endpoint: "correlations/delete",
            id_field: "rule_id",
            request_id_key: "rule_id",
        });
        
        types.insert("widgets", ContentTypeConfig {
            name: "widgets",
            get_endpoint: "widgets/get",
            insert_endpoint: "widgets/insert",
            delete_endpoint: "widgets/delete",
            id_field: "widget_id",
            request_id_key: "widget_id",
        });
        
        types.insert("authentication_settings", ContentTypeConfig {
            name: "authentication_settings",
            get_endpoint: "authentication-settings/get/settings",
            insert_endpoint: "authentication-settings/insert",
            delete_endpoint: "authentication-settings/delete",
            id_field: "name",
            request_id_key: "name",
        });
        
        // Example for future content types:
        // types.insert("incidents", ContentTypeConfig {
        //     name: "incidents",
        //     get_endpoint: "incidents/get_incidents",
        //     insert_endpoint: "incidents/insert",
        //     delete_endpoint: "incidents/delete",
        //     id_field: "incident_id",
        //     request_id_key: "incident_id",
        // });
        
        Self { types }
    }
    
    /// Get configuration for a content type
    pub fn get(&self, content_type: &str) -> Option<&ContentTypeConfig> {
        self.types.get(content_type)
    }
    
    /// Get all supported content type names
    pub fn get_all_types(&self) -> Vec<&'static str> {
        self.types.keys().copied().collect()
    }
    
    /// Check if a content type is supported
    pub fn is_supported(&self, content_type: &str) -> bool {
        self.types.contains_key(content_type)
    }
    
    /// Validate content type (supports both singular and plural forms)
    pub fn get_all_content_types(&self) -> Vec<String> {
        self.types.keys().map(|k| k.to_string()).collect()
    }
    
    pub fn validate_content_type(&self, content_type: &str) -> Result<String, String> {
        // Check exact match first
        if self.is_supported(content_type) {
            return Ok(content_type.to_string());
        }
        
        // Check alternative forms
        let normalized = match content_type {
            "dashboard" => "dashboards",
            "bioc" => "biocs", 
            "correlation_search" => "correlation_searches",
            "widget" => "widgets",
            "authentication_setting" => "authentication_settings",
            _ => return Err(format!("Unsupported content type: {}", content_type)),
        };
        
        if self.is_supported(normalized) {
            Ok(normalized.to_string())
        } else {
            Err(format!("Unsupported content type: {}", content_type))
        }
    }
}

impl Default for ContentTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}