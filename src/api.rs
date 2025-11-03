use anyhow::{Result, Context};
use reqwest::{Client, Response};
use serde_json::Value;

use crate::config::ModuleConfig;
use crate::types::XsiamObject;
use crate::zip_safety;
use crate::modules::{ContentTypeDefinition, PullStrategy};

pub struct ModuleClient {
    client: Client,
    fqdn: String,
    api_key: String,
    api_key_id: String,
    base_api_path: String,
}

impl ModuleClient {
    pub fn new(config: ModuleConfig, base_api_path: &str) -> Self {
        let client = Client::new();
        Self {
            client,
            fqdn: config.fqdn,
            api_key: config.api_key,
            api_key_id: config.api_key_id,
            base_api_path: base_api_path.to_string(),
        }
    }

    // Future push feature - create or update objects on platform
    #[allow(dead_code)]
    pub async fn create_or_update_object(&self, object: &XsiamObject, content_def: &ContentTypeDefinition) -> Result<()> {
        let url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, content_def.get_endpoint);

        // Convert XsiamObject to API format by extracting the content field
        let api_payload = object.content.clone();

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&api_payload)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {url}"))?;

        self.handle_response(response, &format!("create/update {}", object.content_type)).await
    }

    // Future delete feature - remove objects from platform
    #[allow(dead_code)]
    pub async fn delete_object(&self, object: &XsiamObject, content_def: &crate::modules::ContentTypeDefinition) -> Result<()> {
        let url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, content_def.get_endpoint);

        // Build request data using the content type's ID field dynamically
        let mut request_map = serde_json::Map::new();
        request_map.insert(content_def.id_field.to_string(), serde_json::json!(&object.id));
        let request_data = serde_json::json!({
            "request_data": request_map
        });

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&request_data)
            .send()
            .await
            .with_context(|| format!("Failed to send delete request to {url}"))?;

        self.handle_response(response, &format!("delete {}", object.content_type)).await
    }

    // Future feature - comprehensive endpoint testing
    #[allow(dead_code)]
    pub async fn test_all_endpoints(&self, content_types: &[crate::modules::ContentTypeDefinition]) -> Result<()> {
        println!("Testing API connectivity...\n");
        
        let mut successful_endpoints = 0;
        let total_endpoints = content_types.len();
        
        for content_def in content_types {
            let url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, content_def.get_endpoint);
            
            print!("Testing {:<25} ", format!("{}:", content_def.name));
            
            match self.test_single_endpoint(content_def.name, &url).await {
                Ok((status, _count, _sample_name)) => {
                    match status.as_str() {
                        "200" => {
                            println!("SUCCESS: {status}");
                            successful_endpoints += 1;
                        }
                        _ => {
                            println!("WARNING: {status} - Endpoint responded but may have issues");
                        }
                    }
                }
                Err(e) => {
                    println!("FAILED - {e}");
                }
            }
        }
        
        println!("\nConnectivity Test Summary:");
        println!("  Successful: {successful_endpoints}/{total_endpoints}");
        println!("  Failed:     {}/{}", total_endpoints - successful_endpoints, total_endpoints);
        
        if successful_endpoints == total_endpoints {
            println!("  All endpoints are accessible!");
        } else if successful_endpoints > 0 {
            println!("  WARNING: Some endpoints may not be available on this XSIAM instance");
        } else {
            println!("  ERROR: No endpoints are responding - check configuration and network connectivity");
        }
        
        Ok(())
    }
    
    // Helper for test_all_endpoints
    #[allow(dead_code)]
    async fn test_single_endpoint(&self, content_type: &str, url: &str) -> Result<(String, usize, String)> {
        // Debug: Add endpoint information for troubleshooting
        // // println!("  Debug: Testing {} at {}", content_type, url);
        let response = match content_type {
            "incidents" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.api_key_id)
                    .header("Authorization", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {
                            "filters": [],
                            "search_from": 0,
                            "search_to": 1,
                            "sort": {
                                "field": "creation_time",
                                "keyword": "desc"
                            }
                        }
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {url}"))?
            }
            "correlation_searches" | "biocs" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.api_key_id)
                    .header("Authorization", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {url}"))?
            }
            "widgets" | "authentication_settings" | "scripts" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.api_key_id)
                    .header("Authorization", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {url}"))?
            }
            "dashboards" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.api_key_id)
                    .header("Authorization", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {url}"))?
            }
            _ => {
                return Err(anyhow::anyhow!("Unknown content type: {}", content_type));
            }
        };
        
        let status = response.status().as_u16().to_string();
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP {}", status));
        }
        
        let json: Value = response.json().await
            .with_context(|| "Failed to parse JSON response")?;
        
        let (_objects, count, sample_name) = self.extract_test_data(content_type, &json)?;
        
        Ok((status, count, sample_name))
    }
    
    // Helper for test_single_endpoint
    #[allow(dead_code)]
    fn extract_test_data(&self, content_type: &str, json: &Value) -> Result<(Vec<Value>, usize, String)> {
        let objects = match content_type {
            "incidents" => {
                if let Some(incidents) = json.get("reply").and_then(|r| r.get("incidents")).and_then(|i| i.as_array()) {
                    incidents.clone()
                } else {
                    vec![]
                }
            }
            "correlation_searches" => {
                if let Some(correlation_rules) = json.get("reply").and_then(|r| r.as_array()) {
                    correlation_rules.clone()
                } else {
                    vec![]
                }
            }
            "biocs" => {
                if let Some(biocs) = json.get("reply").and_then(|r| r.as_array()) {
                    biocs.clone()
                } else {
                    vec![]
                }
            }
            "widgets" => {
                if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                    if let Some(first_obj) = objects.first() {
                        if let Some(widgets_data) = first_obj.get("widgets_data").and_then(|w| w.as_array()) {
                            widgets_data.clone()
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            }
            "dashboards" => {
                if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                    if let Some(first_obj) = objects.first() {
                        if let Some(dashboards_data) = first_obj.get("dashboards_data").and_then(|d| d.as_array()) {
                            dashboards_data.clone()
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            }
            "authentication_settings" => {
                if let Some(reply_array) = json.get("reply").and_then(|r| r.as_array()) {
                    reply_array.clone()
                } else {
                    vec![]
                }
            }
            "scripts" => {
                if let Some(scripts) = json.get("reply").and_then(|r| r.get("scripts")).and_then(|s| s.as_array()) {
                    scripts.clone()
                } else {
                    vec![]
                }
            }
            _ => vec![]
        };
        
        let count = objects.len();
        let sample_name = if !objects.is_empty() {
            match content_type {
                "widgets" => {
                    objects[0].get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unnamed Widget")
                        .to_string()
                }
                "dashboards" => {
                    objects[0].get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unnamed Dashboard")
                        .to_string()
                }
                "authentication_settings" => {
                    objects[0].get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unnamed Setting")
                        .to_string()
                }
                "scripts" => {
                    objects[0].get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unnamed Script")
                        .to_string()
                }
                _ => {
                    objects[0].get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unnamed Object")
                        .to_string()
                }
            }
        } else {
            String::new()
        };
        
        Ok((objects, count, sample_name))
    }


    pub async fn get_object_by_id(&self, content_def: &ContentTypeDefinition, id: &str) -> Result<XsiamObject> {
        // For AppSec and other modules without filter support, pull all items and search client-side
        // For XSIAM with filter support, use the existing content type-specific pull logic
        let objects = self.pull_content_type(content_def).await?;
        
        // Find the specific object by ID
        for object in objects {
            // Check both id field and the content type's id_field in content
            let id_field_match = object.content.get(content_def.id_field)
                .and_then(|v| {
                    if v.is_string() {
                        v.as_str().map(|s| s.to_string())
                    } else if v.is_i64() {
                        Some(v.as_i64().unwrap().to_string())
                    } else {
                        None
                    }
                })
                .map(|field_id| field_id == id)
                .unwrap_or(false);
            
            if object.id == id || id_field_match {
                return Ok(object);
            }
        }

        // If no matching object found, return error (this will show as "NEW")
        Err(anyhow::anyhow!("Object with ID {} not found in {} response", id, content_def.name))
    }


    // Helper for API response handling
    #[allow(dead_code)]
    async fn handle_response(&self, response: Response, operation: &str) -> Result<()> {
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            Err(anyhow::anyhow!(
                "API {} failed with status {}: {}",
                operation,
                status,
                error_text
            ))
        }
    }

    pub async fn test_connectivity(&self) -> Result<()> {
        // Simple connectivity test using a basic endpoint
        let url = format!("https://{}{}/", self.fqdn, self.base_api_path);
        
        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}", self.fqdn))?;

        if response.status().is_client_error() && response.status().as_u16() == 401 {
            return Err(anyhow::anyhow!("Authentication failed - check API keys"));
        }

        Ok(())
    }




    // Future delete feature - remove objects by ID
    #[allow(dead_code)]
    pub async fn delete_object_by_id(&self, id: &str, content_def: &ContentTypeDefinition) -> Result<()> {
        let url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, content_def.get_endpoint);

        // Build request data using the content type's ID field dynamically
        let mut request_map = serde_json::Map::new();
        request_map.insert(content_def.id_field.to_string(), serde_json::json!(id));
        let request_data = serde_json::json!({
            "request_data": request_map
        });

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_data)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {url}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "API request failed with status: {}\nResponse: {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    #[allow(dead_code)]
    async fn get_scripts_with_content(&self) -> Result<Vec<XsiamObject>> {
        let list_url = format!("https://{}/public_api/v1/scripts/get_scripts", self.fqdn);
        
        let response = self.client
            .post(&list_url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "request_data": {}
            }))
            .send()
            .await
            .with_context(|| format!("Failed to send request to {list_url}"))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "API request failed with status: {}",
                response.status()
            ));
        }

        let json_response: Value = response.json()
            .await
            .context("Failed to parse API response as JSON")?;

        let scripts_list = json_response
            .get("reply")
            .and_then(|r| r.get("scripts"))
            .and_then(|s| s.as_array())
            .ok_or_else(|| anyhow::anyhow!("Expected reply.scripts array in response"))?;

        let mut script_objects = Vec::new();

        for script_meta in scripts_list {
            let script_name = script_meta
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| anyhow::anyhow!("Script missing name field"))?;

            let script_id = script_meta
                .get("script_id")
                .and_then(|id| id.as_str())
                .unwrap_or(script_name)
                .to_string();

            match self.get_single_script_content(script_name).await {
                Ok(yaml_content) => {
                    let mut content_map = std::collections::HashMap::new();
                    
                    if let Ok(yaml_value) = serde_yaml::from_str::<serde_yaml::Value>(&yaml_content) {
                        if let Ok(json_value) = serde_json::to_value(&yaml_value) {
                            if let Some(obj) = json_value.as_object() {
                                for (key, value) in obj {
                                    if key != "name" && key != "description" && key != "script_id" {
                                        content_map.insert(key.clone(), value.clone());
                                    }
                                }
                            }
                        }
                    }
                    
                    content_map.insert("script_id".to_string(), serde_json::json!(script_id.clone()));
                    
                    if let Some(modification_date) = script_meta.get("modification_date") {
                        content_map.insert("modification_date".to_string(), modification_date.clone());
                    }
                    if let Some(windows_supported) = script_meta.get("windows_supported") {
                        content_map.insert("windows_supported".to_string(), windows_supported.clone());
                    }
                    if let Some(linux_supported) = script_meta.get("linux_supported") {
                        content_map.insert("linux_supported".to_string(), linux_supported.clone());
                    }
                    if let Some(macos_supported) = script_meta.get("macos_supported") {
                        content_map.insert("macos_supported".to_string(), macos_supported.clone());
                    }
                    if let Some(is_high_risk) = script_meta.get("is_high_risk") {
                        content_map.insert("is_high_risk".to_string(), is_high_risk.clone());
                    }
                    if let Some(script_uid) = script_meta.get("script_uid") {
                        content_map.insert("script_uid".to_string(), script_uid.clone());
                    }

                    let mut metadata = crate::types::ObjectMetadata::default();
                    if let Some(created_by) = script_meta.get("created_by").and_then(|v| v.as_str()) {
                        metadata.created_by = created_by.to_string();
                    }
                    if let Some(modification_date) = script_meta.get("modification_date").and_then(|v| v.as_i64()) {
                        let seconds = if modification_date > 10000000000 {
                            modification_date / 1000
                        } else {
                            modification_date
                        };
                        metadata.updated_at = chrono::DateTime::from_timestamp(seconds, 0);
                    }

                    let description = script_meta
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();

                    let xsiam_obj = XsiamObject {
                        id: script_id,
                        name: Some(script_name.to_string()),
                        description,
                        content_type: "scripts".to_string(),
                        metadata,
                        tenant_id: None,
                        content: content_map,
                    };
                    script_objects.push(xsiam_obj);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to download script '{script_name}': {e}");
                }
            }
        }

        Ok(script_objects)
    }

    #[allow(dead_code)]
    async fn get_single_script_content(&self, script_name: &str) -> Result<String> {
        let get_url = format!("https://{}/public_api/v1/scripts/get", self.fqdn);
        
        let response = self.client
            .post(&get_url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "request_data": {
                    "filter": {
                        "field": "name",
                        "value": script_name
                    }
                }
            }))
            .send()
            .await
            .with_context(|| format!("Failed to download script '{script_name}'"))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to download script '{}': HTTP {}",
                script_name,
                response.status()
            ));
        }

        let zip_bytes = response.bytes()
            .await
            .context("Failed to read ZIP response")?;

        let yaml_content = zip_safety::extract_yaml_from_zip(&zip_bytes)
            .with_context(|| format!("Failed to extract YAML from script '{script_name}' ZIP"))?;

        Ok(yaml_content)
    }

    #[allow(dead_code)]
    fn parse_api_response(&self, json: &Value, content_type: &str) -> Result<Vec<XsiamObject>> {
        let mut objects = Vec::new();

        // Handle different response formats based on content type
        let items = match content_type {
            "widgets" => {
                // Handle nested widget response: {"objects": [{"widgets_data": [...]}]}
                if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                    if let Some(first_obj) = objects.first() {
                        if let Some(widgets_data) = first_obj.get("widgets_data").and_then(|d| d.as_array()) {
                            widgets_data
                        } else {
                            return Err(anyhow::anyhow!("Expected widgets_data in objects[0]: {}", json));
                        }
                    } else {
                        return Ok(Vec::new()); // Empty objects array
                    }
                } else if let Some(widgets_data) = json.get("widgets_data").and_then(|d| d.as_array()) {
                    widgets_data
                } else {
                    return Err(anyhow::anyhow!("Expected widgets_data array in response: {}", json));
                }
            }
            "dashboards" => {
                // Handle nested dashboard response: {"objects": [{"dashboards_data": [...]}]}
                if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
                    if let Some(first_obj) = objects.first() {
                        if let Some(dashboards_data) = first_obj.get("dashboards_data").and_then(|d| d.as_array()) {
                            dashboards_data
                        } else {
                            return Err(anyhow::anyhow!("Expected dashboards_data in objects[0]: {}", json));
                        }
                    } else {
                        return Ok(Vec::new()); // Empty objects array
                    }
                } else if let Some(dashboards_data) = json.get("dashboards_data").and_then(|d| d.as_array()) {
                    dashboards_data
                } else {
                    return Err(anyhow::anyhow!("Expected dashboards_data array in response: {}", json));
                }
            }
            "authentication_settings" => {
                // Handle authentication settings response: {"reply": [...]}
                if let Some(reply_array) = json.get("reply").and_then(|r| r.as_array()) {
                    reply_array
                } else {
                    return Err(anyhow::anyhow!("Expected reply array in authentication_settings response: {}", json));
                }
            }
            _ => {
                // Handle other content types with existing logic
                if let Some(array) = json.as_array() {
                    array
                } else if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
                    data
                } else if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                    items
                } else if let Some(reply) = json.get("reply") {
                    // Handle XSIAM response format {"reply": {"data": [...]}}
                    if let Some(data) = reply.get("data").and_then(|d| d.as_array()) {
                        data
                    } else {
                        // Empty reply or no data
                        return Ok(objects);
                    }
                } else if let Some(objects_array) = json.get("objects").and_then(|o| o.as_array()) {
                    // Handle XSIAM correlation rules format {"objects": [...], "objects_count": n}
                    objects_array
                } else {
                    return Err(anyhow::anyhow!("Unexpected API response format: {}", json));
                }
            }
        };

        for item in items.iter() {
            let object = XsiamObject::from_api_response(item, content_type)?;
            objects.push(object);
        }

        Ok(objects)
    }
    
    /// Pull content using ContentTypeDefinition - supports all pull strategies
    pub async fn pull_content_type(&self, content_def: &ContentTypeDefinition) -> Result<Vec<XsiamObject>> {
        match &content_def.pull_strategy {
            PullStrategy::JsonCollection => {
                self.pull_json_collection(content_def).await
            }
            PullStrategy::Paginated { page_param, page_size_param, page_size } => {
                self.pull_paginated(content_def, page_param, page_size_param, *page_size).await
            }
            PullStrategy::ZipArtifact { metadata_endpoint, download_endpoint, metadata_response_path, download_filter_field } => {
                self.pull_zip_artifact(content_def, metadata_endpoint, download_endpoint, metadata_response_path, download_filter_field).await
            }
            PullStrategy::ScriptCode { list_endpoint, code_endpoint, list_response_path, uid_field } => {
                self.pull_script_code(content_def, list_endpoint, code_endpoint, list_response_path, uid_field).await
            }
        }
    }
    
    /// Pull JSON collection - single API call
    async fn pull_json_collection(&self, content_def: &ContentTypeDefinition) -> Result<Vec<XsiamObject>> {
        let url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, content_def.get_endpoint);
        
        let response = if let Some(body) = &content_def.request_body {
            // POST request with body
            self.client
                .post(&url)
                .header("x-xdr-auth-id", &self.api_key_id)
                .header("Authorization", &self.api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(body)
                .send()
                .await
                .with_context(|| format!("Failed to send request to {url}"))?
        } else {
            // GET request
            self.client
                .get(&url)
                .header("x-xdr-auth-id", &self.api_key_id)
                .header("Authorization", &self.api_key)
                .header("Accept", "application/json")
                .send()
                .await
                .with_context(|| format!("Failed to send request to {url}"))?
        };
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("API request failed with status: {}", response.status()));
        }
        
        let json: Value = response.json().await.context("Failed to parse JSON response")?;
        self.extract_items_from_response(&json, content_def)
    }
    
    /// Pull paginated content - multiple API calls
    async fn pull_paginated(&self, content_def: &ContentTypeDefinition, page_param: &str, page_size_param: &str, page_size: usize) -> Result<Vec<XsiamObject>> {
        let mut all_objects = Vec::new();
        let mut page = 1;
        
        loop {
            let url = format!("https://{}{}/{}?{}={}&{}={}", 
                self.fqdn, self.base_api_path, content_def.get_endpoint,
                page_param, page,
                page_size_param, page_size
            );
            
            let response = self.client
                .get(&url)
                .header("x-xdr-auth-id", &self.api_key_id)
                .header("Authorization", &self.api_key)
                .header("Accept", "application/json")
                .send()
                .await
                .with_context(|| format!("Failed to send paginated request to {url}"))?;
            
            if !response.status().is_success() {
                return Err(anyhow::anyhow!("API request failed with status: {}", response.status()));
            }
            
            let json: Value = response.json().await.context("Failed to parse JSON response")?;
            let objects = self.extract_items_from_response(&json, content_def)?;
            
            if objects.is_empty() {
                break;
            }
            
            all_objects.extend(objects);
            page += 1;
        }
        
        Ok(all_objects)
    }
    
    /// Pull ZIP artifacts - two-step process (metadata list + individual downloads)
    async fn pull_zip_artifact(&self, content_def: &ContentTypeDefinition, metadata_endpoint: &str, download_endpoint: &str, metadata_response_path: &str, download_filter_field: &str) -> Result<Vec<XsiamObject>> {
        let list_url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, metadata_endpoint);
        
        let response = self.client
            .post(&list_url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&serde_json::json!({"request_data": {}}))
            .send()
            .await
            .with_context(|| format!("Failed to send request to {list_url}"))?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("API request failed with status: {}", response.status()));
        }
        
        let json_response: Value = response.json().await.context("Failed to parse API response as JSON")?;
        
        let scripts_list = self.extract_value_by_path(&json_response, metadata_response_path)?
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected array at path {}", metadata_response_path))?;
        
        let mut script_objects = Vec::new();
        
        for script_meta in scripts_list {
            let script_name = script_meta
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| anyhow::anyhow!("Script missing name field"))?;
            
            let script_id = script_meta
                .get(content_def.id_field)
                .and_then(|id| id.as_str())
                .unwrap_or(script_name)
                .to_string();
            
            match self.download_zip_artifact(download_endpoint, download_filter_field, script_name).await {
                Ok(yaml_content) => {
                    let mut content_map = std::collections::HashMap::new();
                    
                    if let Ok(yaml_value) = serde_yaml::from_str::<serde_yaml::Value>(&yaml_content) {
                        if let Ok(json_value) = serde_json::to_value(&yaml_value) {
                            if let Some(obj) = json_value.as_object() {
                                for (key, value) in obj {
                                    if key != "name" && key != "description" && key != content_def.id_field {
                                        content_map.insert(key.clone(), value.clone());
                                    }
                                }
                            }
                        }
                    }
                    
                    content_map.insert(content_def.id_field.to_string(), serde_json::json!(script_id.clone()));
                    
                    for (key, value) in script_meta.as_object().unwrap_or(&serde_json::Map::new()) {
                        if key != "name" && key != "description" {
                            content_map.insert(key.clone(), value.clone());
                        }
                    }
                    
                    let mut metadata = crate::types::ObjectMetadata::default();
                    if let Some(created_by) = script_meta.get("created_by").and_then(|v| v.as_str()) {
                        metadata.created_by = created_by.to_string();
                    }
                    if let Some(modification_date) = script_meta.get("modification_date").and_then(|v| v.as_i64()) {
                        let seconds = if modification_date > 10000000000 {
                            modification_date / 1000
                        } else {
                            modification_date
                        };
                        metadata.updated_at = chrono::DateTime::from_timestamp(seconds, 0);
                    }
                    
                    let description = script_meta
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();
                    
                    let xsiam_obj = XsiamObject {
                        id: script_id,
                        name: Some(script_name.to_string()),
                        description,
                        content_type: content_def.name.to_string(),
                        metadata,
                        tenant_id: None,
                        content: content_map,
                    };
                    script_objects.push(xsiam_obj);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to download {} '{}': {}", content_def.name, script_name, e);
                }
            }
        }
        
        Ok(script_objects)
    }
    
    /// Download a ZIP artifact
    async fn download_zip_artifact(&self, download_endpoint: &str, filter_field: &str, filter_value: &str) -> Result<String> {
        let get_url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, download_endpoint);
        
        let response = self.client
            .post(&get_url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "request_data": {
                    "filters": [{
                        "field": filter_field,
                        "value": filter_value
                    }]
                }
            }))
            .send()
            .await
            .with_context(|| format!("Failed to download artifact '{filter_value}'"))?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to download artifact '{}': HTTP {}", filter_value, response.status()));
        }
        
        let zip_bytes = response.bytes().await.context("Failed to read ZIP response")?;
        let yaml_content = zip_safety::extract_yaml_from_zip(&zip_bytes)
            .with_context(|| format!("Failed to extract YAML from artifact '{filter_value}' ZIP"))?;
        
        Ok(yaml_content)
    }
    
    /// Pull script code - two-step process (list scripts + fetch code by UID)
    async fn pull_script_code(&self, content_def: &ContentTypeDefinition, list_endpoint: &str, code_endpoint: &str, list_response_path: &str, uid_field: &str) -> Result<Vec<XsiamObject>> {
        let list_url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, list_endpoint);
        
        let response = self.client
            .post(&list_url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&serde_json::json!({"request_data": {}}))
            .send()
            .await
            .with_context(|| format!("Failed to send request to {list_url}"))?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("API request failed with status: {}", response.status()));
        }
        
        let json_response: Value = response.json().await.context("Failed to parse API response as JSON")?;
        
        let scripts_list = self.extract_value_by_path(&json_response, list_response_path)?
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected array at path {}", list_response_path))?;
        
        let mut script_objects = Vec::new();
        
        for script_meta in scripts_list {
            let script_uid = script_meta
                .get(uid_field)
                .and_then(|uid| uid.as_str())
                .ok_or_else(|| anyhow::anyhow!("Script missing {} field", uid_field))?;
            
            let script_name = script_meta
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(script_uid);
            
            match self.get_script_code(code_endpoint, script_uid).await {
                Ok(script_code) => {
                    let mut content_map = std::collections::HashMap::new();
                    
                    // Store the script code with newlines properly converted
                    content_map.insert("code".to_string(), serde_json::json!(script_code));
                    
                    // Add all metadata fields except name, description, and uid
                    for (key, value) in script_meta.as_object().unwrap_or(&serde_json::Map::new()) {
                        if key != "name" && key != "description" && key != uid_field {
                            content_map.insert(key.clone(), value.clone());
                        }
                    }
                    
                    let mut metadata = crate::types::ObjectMetadata::default();
                    if let Some(created_by) = script_meta.get("created_by").and_then(|v| v.as_str()) {
                        metadata.created_by = created_by.to_string();
                    }
                    if let Some(modification_date) = script_meta.get("modification_date").and_then(|v| v.as_i64()) {
                        let seconds = if modification_date > 10000000000 {
                            modification_date / 1000
                        } else {
                            modification_date
                        };
                        metadata.updated_at = chrono::DateTime::from_timestamp(seconds, 0);
                    }
                    
                    let description = script_meta
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();
                    
                    let xsiam_obj = XsiamObject {
                        id: script_uid.to_string(),
                        name: Some(script_name.to_string()),
                        description,
                        content_type: content_def.name.to_string(),
                        metadata,
                        tenant_id: None,
                        content: content_map,
                    };
                    script_objects.push(xsiam_obj);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to get code for script '{script_name}': {e}");
                }
            }
        }
        
        Ok(script_objects)
    }
    
    /// Get script code by UID - returns code with escaped newlines converted to actual newlines
    async fn get_script_code(&self, code_endpoint: &str, script_uid: &str) -> Result<String> {
        let code_url = format!("https://{}{}/{}", self.fqdn, self.base_api_path, code_endpoint);
        
        let response = self.client
            .post(&code_url)
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "request_data": {
                    "script_uid": script_uid
                }
            }))
            .send()
            .await
            .with_context(|| format!("Failed to get script code for UID '{script_uid}'"))?;
        
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to get script code for UID '{}': HTTP {}", script_uid, response.status()));
        }
        
        let json: Value = response.json().await.context("Failed to parse script code response")?;
        
        let script_code = json.get("reply")
            .and_then(|r| r.as_str())
            .ok_or_else(|| anyhow::anyhow!("Script code response missing 'reply' field"))?;
        
        // Convert escaped newlines (\n) to actual newlines for readability
        let code_with_newlines = script_code.replace("\\n", "\n");
        
        Ok(code_with_newlines)
    }
    
    /// Extract items from JSON response using response_path
    /// Logs warnings when response structure doesn't match expectations to help distinguish
    /// between "no data" vs "API structure changed"
    fn extract_items_from_response(&self, json: &Value, content_def: &ContentTypeDefinition) -> Result<Vec<XsiamObject>> {
        let items = if let Some(path) = content_def.response_path {
            // Try to extract from path - log warning if path doesn't exist
            match self.extract_value_by_path(json, path) {
                Ok(value) => {
                    match value.as_array() {
                        Some(arr) => arr,
                        None => {
                            // Path exists but isn't an array - possible API change
                            eprintln!("WARNING: Response path '{}' for {} exists but is not an array (found {}). Endpoint may have changed structure or returned error.",
                                path, content_def.name, value.as_str().unwrap_or("non-string value"));
                            return Ok(Vec::new());
                        }
                    }
                },
                Err(_) => {
                    // Path doesn't exist - could be no data OR API structure changed
                    eprintln!("WARNING: Response path '{}' not found for {}. This could mean: (1) endpoint has no data, or (2) API response structure has changed. Verify endpoint is working correctly.",
                        path, content_def.name);
                    return Ok(Vec::new());
                }
            }
        } else {
            // No path specified - expect array at root
            match json.as_array() {
                Some(arr) => arr,
                None => {
                    // Root isn't an array - possible API change
                    eprintln!("WARNING: Expected array at root for {} but found {}. API response structure may have changed.",
                        content_def.name, json.get("error").and_then(|e| e.as_str()).unwrap_or("non-array response"));
                    return Ok(Vec::new());
                }
            }
        };
        
        let mut objects = Vec::new();
        for item in items {
            let object = XsiamObject::from_api_response(item, content_def.name)?;
            objects.push(object);
        }
        
        Ok(objects)
    }
    
    /// Extract value from JSON using dot-notation path (e.g., "reply.scripts", "objects[0].dashboards_data")
    fn extract_value_by_path<'a>(&self, json: &'a Value, path: &str) -> Result<&'a Value> {
        let mut current = json;
        
        for segment in path.split('.') {
            if segment.contains('[') && segment.ends_with(']') {
                let parts: Vec<&str> = segment.split('[').collect();
                let field = parts[0];
                let index_str = parts[1].trim_end_matches(']');
                let index: usize = index_str.parse()
                    .with_context(|| format!("Invalid array index: {index_str}"))?;
                
                if !field.is_empty() {
                    current = current.get(field)
                        .ok_or_else(|| anyhow::anyhow!("Path segment '{}' not found", field))?;
                }
                
                current = current.get(index)
                    .ok_or_else(|| anyhow::anyhow!("Array index {} not found", index))?;
            } else {
                current = current.get(segment)
                    .ok_or_else(|| anyhow::anyhow!("Path segment '{}' not found", segment))?;
            }
        }
        
        Ok(current)
    }
}
