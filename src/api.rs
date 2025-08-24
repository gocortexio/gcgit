use anyhow::{Result, Context};
use reqwest::{Client, Response};
use serde_json::Value;

use crate::config::XsiamConfig;
use crate::content_types::ContentTypeRegistry;
use crate::types::XsiamObject;

pub struct XsiamClient {
    client: Client,
    config: XsiamConfig,
}

impl XsiamClient {
    pub fn new(config: XsiamConfig) -> Self {
        let client = Client::new();
        Self { client, config }
    }

    pub async fn create_or_update_object(&self, object: &XsiamObject) -> Result<()> {
        let endpoint = self.get_create_endpoint(&object.content_type);
        let url = format!("https://{}/public_api/v1/{}", self.config.fqdn, endpoint);

        // Convert XsiamObject to API format by extracting the content field
        let api_payload = object.content.clone();

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.config.api_key_id)
            .header("Authorization", &self.config.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&api_payload)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {}", url))?;

        self.handle_response(response, &format!("create/update {}", object.content_type)).await
    }

    pub async fn delete_object(&self, object: &XsiamObject) -> Result<()> {
        let endpoint = self.get_delete_endpoint(&object.content_type);
        let url = format!("https://{}/public_api/v1/{}", self.config.fqdn, endpoint);

        // Get the correct request data format based on content type
        let registry = crate::content_types::ContentTypeRegistry::new();
        let request_data = if let Some(config) = registry.get(&object.content_type) {
            config.get_request_data(&object.id)
        } else {
            // Fallback for unknown content types
            serde_json::json!({
                "request_data": {
                    "id": object.id
                }
            })
        };

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.config.api_key_id)
            .header("Authorization", &self.config.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&request_data)
            .send()
            .await
            .with_context(|| format!("Failed to send delete request to {}", url))?;

        self.handle_response(response, &format!("delete {}", object.content_type)).await
    }

    pub async fn test_all_endpoints(&self) -> Result<()> {
        println!("🔍 Testing XSIAM API connectivity...\n");
        
        let content_types = ContentTypeRegistry::new().get_all_content_types();
        let mut successful_endpoints = 0;
        let total_endpoints = content_types.len();
        
        for content_type in content_types {
            let endpoint = self.get_endpoint(&content_type);
            let url = format!("https://{}/public_api/v1/{}", self.config.fqdn, endpoint);
            
            print!("Testing {:<25} ", format!("{}:", content_type));
            
            match self.test_single_endpoint(&content_type, &url).await {
                Ok((status, _count, _sample_name)) => {
                    match status.as_str() {
                        "200" => {
                            println!("✅ {}", status);
                            successful_endpoints += 1;
                        }
                        _ => {
                            println!("⚠️  {} - Endpoint responded but may have issues", status);
                        }
                    }
                }
                Err(e) => {
                    println!("❌ FAILED - {}", e);
                }
            }
        }
        
        println!("\n📊 Connectivity Test Summary:");
        println!("  ✅ Successful: {}/{}", successful_endpoints, total_endpoints);
        println!("  ❌ Failed:     {}/{}", total_endpoints - successful_endpoints, total_endpoints);
        
        if successful_endpoints == total_endpoints {
            println!("  🎉 All endpoints are accessible!");
        } else if successful_endpoints > 0 {
            println!("  ⚠️  Some endpoints may not be available on this XSIAM instance");
        } else {
            println!("  🚨 No endpoints are responding - check configuration and network connectivity");
        }
        
        Ok(())
    }
    
    async fn test_single_endpoint(&self, content_type: &str, url: &str) -> Result<(String, usize, String)> {
        // Debug: Add endpoint information for troubleshooting
        // // println!("  Debug: Testing {} at {}", content_type, url);
        let response = match content_type {
            "incidents" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
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
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "correlation_searches" | "biocs" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "widgets" | "authentication_settings" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "dashboards" => {
                self.client
                    .post(url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
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

    pub async fn get_objects(&self, content_type: &str) -> Result<Vec<XsiamObject>> {
        let endpoint = self.get_endpoint(content_type);
        let url = format!("https://{}/public_api/v1/{}", self.config.fqdn, endpoint);

        // Most XSIAM endpoints use POST requests with empty or specific request data
        let response = match content_type {
            "incidents" => {
                self.client
                    .post(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {
                            "limit": 10
                        }
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "correlation_searches" => {
                self.client
                    .post(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "dashboards" => {
                self.client
                    .post(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "biocs" => {
                self.client
                    .post(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "widgets" => {
                self.client
                    .post(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            "authentication_settings" => {
                self.client
                    .post(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&serde_json::json!({
                        "request_data": {}
                    }))
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
            _ => {
                self.client
                    .get(&url)
                    .header("x-xdr-auth-id", &self.config.api_key_id)
                    .header("Authorization", &self.config.api_key)
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request to {}", url))?
            }
        };

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "API request failed with status: {}",
                response.status()
            ));
        }

        let json_response: Value = response.json()
            .await
            .context("Failed to parse API response as JSON")?;

        let objects = self.parse_api_response(&json_response, content_type)?;
        Ok(objects)
    }

    pub async fn get_object_by_id(&self, content_type: &str, id: &str) -> Result<XsiamObject> {
        let endpoint = self.get_endpoint(content_type);
        let url = format!("https://{}/public_api/v1/{}", self.config.fqdn, endpoint);

        // Use POST request with content-type specific filter to get specific object
        let registry = ContentTypeRegistry::new();
        let request_data = if let Some(config) = registry.get(content_type) {
            config.get_request_data(id)
        } else {
            serde_json::json!({
                "request_data": {
                    "id": id
                }
            })
        };

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.config.api_key_id)
            .header("Authorization", &self.config.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&request_data)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {}", url))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "API request failed with status: {}",
                response.status()
            ));
        }

        let json_response: Value = response.json()
            .await
            .context("Failed to parse API response as JSON")?;

        // Parse the response and find the object with matching ID
        let objects = self.parse_api_response(&json_response, content_type)?;
        
        // Find the specific object by ID
        for object in objects {
            // Check both id field and rule_id in content
            let rule_id_match = object.content.get("rule_id")
                .and_then(|v| v.as_i64())
                .map(|r| r.to_string()) == Some(id.to_string());
            
            if object.id == id || rule_id_match {
                return Ok(object);
            }
        }

        // If no matching object found, return error (this will show as "NEW")
        Err(anyhow::anyhow!("Object with ID {} not found in {} response", id, content_type))
    }

    fn get_endpoint(&self, content_type: &str) -> &str {
        let registry = ContentTypeRegistry::new();
        if let Some(config) = registry.get(content_type) {
            config.get_endpoint
        } else {
            "unknown"
        }
    }

    fn get_create_endpoint(&self, content_type: &str) -> &str {
        let registry = ContentTypeRegistry::new();
        if let Some(config) = registry.get(content_type) {
            config.insert_endpoint
        } else {
            "unknown"
        }
    }

    fn get_delete_endpoint(&self, content_type: &str) -> &str {
        let registry = ContentTypeRegistry::new();
        if let Some(config) = registry.get(content_type) {
            config.delete_endpoint
        } else {
            "unknown"
        }
    }

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





    pub async fn test_connectivity(&self) -> Result<Vec<XsiamObject>> {
        // Use correlation searches as a test endpoint since it's most commonly available
        self.get_objects("correlation_searches").await
    }

    pub async fn delete_object_by_id(&self, content_type: &str, id: &str) -> Result<()> {
        let endpoint = self.get_delete_endpoint(content_type);
        let url = format!("https://{}/public_api/v1/{}", self.config.fqdn, endpoint);

        // Get the correct request data format based on content type
        let registry = crate::content_types::ContentTypeRegistry::new();
        let request_data = if let Some(config) = registry.get(content_type) {
            config.get_request_data(id)
        } else {
            // Fallback for unknown content types - try both string and integer IDs
            if let Ok(id_num) = id.parse::<i64>() {
                serde_json::json!({
                    "request_data": {
                        "objects_count": 1,
                        "objects": [id_num]
                    }
                })
            } else {
                serde_json::json!({
                    "request_data": {
                        "id": id
                    }
                })
            }
        };

        let response = self.client
            .post(&url)
            .header("x-xdr-auth-id", &self.config.api_key_id)
            .header("Authorization", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request_data)
            .send()
            .await
            .with_context(|| format!("Failed to send request to {}", url))?;

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
}
