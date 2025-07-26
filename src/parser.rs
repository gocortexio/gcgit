use anyhow::{Result, Context};
use std::fs;
use std::path::Path;

use crate::types::XsiamObject;

pub struct YamlParser;

impl YamlParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse_file(&self, file_path: &str) -> Result<XsiamObject> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path))?;

        let mut object: XsiamObject = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse YAML file: {}", file_path))?;

        // Infer content type from file path if not specified
        if object.content_type.is_empty() {
            object.content_type = self.infer_content_type(file_path)?;
        }

        // Validate the object
        self.validate_object(&object)?;

        Ok(object)
    }

    pub fn write_file(&self, file_path: &str, object: &XsiamObject) -> Result<()> {
        // Ensure directory exists
        if let Some(parent) = Path::new(file_path).parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create a deterministic YAML output with consistent field ordering
        let yaml_content = self.serialize_object_deterministically(object)
            .with_context(|| format!("Failed to serialize object to YAML"))?;

        fs::write(file_path, yaml_content)
            .with_context(|| format!("Failed to write file: {}", file_path))?;

        Ok(())
    }

    pub fn serialize_object_deterministically(&self, object: &XsiamObject) -> Result<String> {
        use serde_yaml::{Mapping, Value as YamlValue};

        let mut yaml_map = Mapping::new();
        
        // Add fields in a specific order to ensure consistency
        yaml_map.insert(YamlValue::String("id".to_string()), YamlValue::String(object.id.clone()));
        yaml_map.insert(YamlValue::String("name".to_string()), YamlValue::String(object.name.clone()));
        yaml_map.insert(YamlValue::String("description".to_string()), YamlValue::String(object.description.clone()));
        yaml_map.insert(YamlValue::String("content_type".to_string()), YamlValue::String(object.content_type.clone()));
        
        // Serialize metadata with consistent ordering
        let metadata_yaml = serde_yaml::to_value(&object.metadata)?;
        yaml_map.insert(YamlValue::String("metadata".to_string()), metadata_yaml);
        
        // Sort content HashMap keys for consistent ordering
        let mut sorted_keys: Vec<_> = object.content.keys().collect();
        sorted_keys.sort();
        
        // Add content fields in alphabetical order
        for key in sorted_keys {
            if let Some(value) = object.content.get(key) {
                let yaml_value = serde_json::to_value(value)
                    .map_err(|e| anyhow::anyhow!("JSON serialisation error: {}", e))
                    .and_then(|json_val| serde_yaml::to_value(json_val)
                        .map_err(|e| anyhow::anyhow!("YAML serialisation error: {}", e)))
                    .unwrap_or(YamlValue::Null);
                yaml_map.insert(YamlValue::String(key.clone()), yaml_value);
            }
        }

        serde_yaml::to_string(&YamlValue::Mapping(yaml_map))
            .with_context(|| "Failed to convert to YAML string")
    }

    /// Compare two XsiamObjects using deterministic serialisation to ensure accurate comparison
    /// Note: This method includes metadata in comparison and is mainly used for debugging
    #[allow(dead_code)]
    pub fn objects_are_equal(&self, obj1: &XsiamObject, obj2: &XsiamObject) -> Result<bool> {
        let serialized1 = self.serialize_object_deterministically(obj1)?;
        let serialized2 = self.serialize_object_deterministically(obj2)?;
        Ok(serialized1 == serialized2)
    }

    /// Compare two XsiamObjects excluding metadata (for logical comparison)
    /// This is the preferred method for determining if objects are functionally different
    pub fn objects_are_logically_equal(&self, obj1: &XsiamObject, obj2: &XsiamObject) -> Result<bool> {
        // Compare basic fields
        if obj1.id != obj2.id || 
           obj1.name != obj2.name || 
           obj1.description != obj2.description || 
           obj1.content_type != obj2.content_type {
            return Ok(false);
        }

        // Compare content using deterministic serialisation
        let content1_yaml = self.serialize_content_deterministically(&obj1.content)?;
        let content2_yaml = self.serialize_content_deterministically(&obj2.content)?;
        
        Ok(content1_yaml == content2_yaml)
    }

    /// Serialize just the content HashMap with deterministic ordering
    fn serialize_content_deterministically(&self, content: &std::collections::HashMap<String, serde_json::Value>) -> Result<String> {
        use serde_yaml::{Mapping, Value as YamlValue};

        let mut yaml_map = Mapping::new();
        
        // Sort content HashMap keys for consistent ordering
        let mut sorted_keys: Vec<_> = content.keys().collect();
        sorted_keys.sort();
        
        // Add content fields in alphabetical order
        for key in sorted_keys {
            if let Some(value) = content.get(key) {
                let yaml_value = serde_json::to_value(value)
                    .map_err(|e| anyhow::anyhow!("JSON serialisation error: {}", e))
                    .and_then(|json_val| serde_yaml::to_value(json_val)
                        .map_err(|e| anyhow::anyhow!("YAML serialisation error: {}", e)))
                    .unwrap_or(YamlValue::Null);
                yaml_map.insert(YamlValue::String(key.clone()), yaml_value);
            }
        }

        serde_yaml::to_string(&YamlValue::Mapping(yaml_map))
            .with_context(|| "Failed to convert content to YAML string")
    }

    pub fn get_local_files(&self, instance_name: &str) -> Result<Vec<String>> {
        let mut files = Vec::new();
        
        let instance_path = Path::new(instance_name);
        if !instance_path.exists() {
            return Ok(files);
        }

        let registry = crate::content_types::ContentTypeRegistry::new();
        let content_types = registry.get_all_types();
        
        for content_type in content_types {
            let type_path = instance_path.join(content_type);
            if type_path.exists() {
                let entries = fs::read_dir(&type_path)
                    .with_context(|| format!("Failed to read directory: {}", type_path.display()))?;

                for entry in entries {
                    let entry = entry.context("Failed to read directory entry")?;
                    let path = entry.path();
                    
                    if path.extension().map_or(false, |ext| ext == "yaml" || ext == "yml") {
                        if let Some(path_str) = path.to_str() {
                            files.push(path_str.to_string());
                        }
                    }
                }
            }
        }

        Ok(files)
    }

    fn infer_content_type(&self, file_path: &str) -> Result<String> {
        let path = Path::new(file_path);
        
        if let Some(parent) = path.parent() {
            if let Some(parent_name) = parent.file_name() {
                if let Some(parent_str) = parent_name.to_str() {
                    // Use content type registry to validate
                    let registry = crate::content_types::ContentTypeRegistry::new();
                    if registry.is_supported(parent_str) {
                        return Ok(parent_str.to_string());
                    }
                }
            }
        }

        Err(anyhow::anyhow!("Unable to infer content type from file path: {}", file_path))
    }

    fn validate_object(&self, object: &XsiamObject) -> Result<()> {
        if object.id.is_empty() {
            return Err(anyhow::anyhow!("Object ID is required"));
        }

        if object.name.is_empty() {
            return Err(anyhow::anyhow!("Object name is required"));
        }

        if object.content_type.is_empty() {
            return Err(anyhow::anyhow!("Content type is required"));
        }

        // Validate content type using registry - accept both singular and plural forms
        let registry = crate::content_types::ContentTypeRegistry::new();
        match registry.validate_content_type(&object.content_type) {
            Ok(_) => {},
            Err(e) => return Err(anyhow::anyhow!("{}", e)),
        }

        Ok(())
    }
}
