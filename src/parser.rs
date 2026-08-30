// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, Result};
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
            .with_context(|| format!("Failed to read file: {file_path}"))?;

        let mut object: XsiamObject = serde_yaml_ng::from_str(&content)
            .with_context(|| format!("Failed to parse YAML file: {file_path}"))?;

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
        let yaml_content = self
            .serialize_object_deterministically(object)
            .with_context(|| "Failed to serialize object to YAML".to_string())?;

        fs::write(file_path, yaml_content)
            .with_context(|| format!("Failed to write file: {file_path}"))?;

        Ok(())
    }

    pub fn serialize_object_deterministically(&self, object: &XsiamObject) -> Result<String> {
        use serde_yaml_ng::{Mapping, Value as YamlValue};

        let mut yaml_map = Mapping::new();

        // Add fields in a specific order to ensure consistency
        yaml_map.insert(
            YamlValue::String("id".to_string()),
            YamlValue::String(object.id.clone()),
        );
        if let Some(name) = &object.name {
            yaml_map.insert(
                YamlValue::String("name".to_string()),
                YamlValue::String(name.clone()),
            );
        }
        yaml_map.insert(
            YamlValue::String("description".to_string()),
            YamlValue::String(object.description.clone()),
        );
        yaml_map.insert(
            YamlValue::String("content_type".to_string()),
            YamlValue::String(object.content_type.clone()),
        );

        // Serialize metadata with consistent ordering
        let metadata_yaml = serde_yaml_ng::to_value(&object.metadata)?;
        yaml_map.insert(YamlValue::String("metadata".to_string()), metadata_yaml);

        // Sort content HashMap keys alphabetically for deterministic YAML output.
        // Only the top-level key order is normalised. Values are written exactly as
        // the API returned them: array order is part of the configuration for fields
        // such as correlation `suppression_fields` and BIOC `mitre_technique_id_and_name`,
        // so reordering them would mean the stored YAML no longer matches the platform.
        let mut sorted_keys: Vec<_> = object.content.keys().collect();
        sorted_keys.sort();

        // Add content fields in alphabetical order
        for key in sorted_keys {
            if let Some(value) = object.content.get(key) {
                let json_val = serde_json::to_value(value)
                    .map_err(|e| anyhow::anyhow!("JSON serialisation error: {e}"))?;
                let yaml_value = serde_yaml_ng::to_value(json_val)
                    .map_err(|e| anyhow::anyhow!("YAML serialisation error: {e}"))
                    .unwrap_or(YamlValue::Null);
                yaml_map.insert(YamlValue::String(key.clone()), yaml_value);
            }
        }

        serde_yaml_ng::to_string(&YamlValue::Mapping(yaml_map))
            .with_context(|| "Failed to convert to YAML string")
    }

    /// Compare two objects by the exact bytes each would be written as.
    ///
    /// diff must predict pull. Comparing a subset of the object meant diff could
    /// report no difference for something a subsequent pull then rewrote, and
    /// show_object_differences printed a note acknowledging the contradiction
    /// rather than resolving it. If a field produces noise here it also produces
    /// noise in Git, and the fix is to stop storing that field.
    pub fn objects_are_logically_equal(
        &self,
        obj1: &XsiamObject,
        obj2: &XsiamObject,
    ) -> Result<bool> {
        let a = self.serialize_object_deterministically(obj1)?;
        let b = self.serialize_object_deterministically(obj2)?;
        Ok(a == b)
    }

    /// Get all local YAML files for specific content types in a module directory
    ///
    /// # Arguments
    /// * `module_dir` - Path to module directory (e.g., "instance/xsiam" or "instance/appsec")
    /// * `content_type_names` - List of content type subdirectory names to search
    pub fn get_local_files(
        &self,
        module_dir: &str,
        content_type_names: &[&str],
    ) -> Result<Vec<String>> {
        let mut files = Vec::new();

        let module_path = Path::new(module_dir);
        if !module_path.exists() {
            return Ok(files);
        }

        for content_type in content_type_names {
            let type_path = module_path.join(content_type);
            if type_path.exists() {
                let entries = fs::read_dir(&type_path).with_context(|| {
                    format!("Failed to read directory: {}", type_path.display())
                })?;

                for entry in entries {
                    let entry = entry.context("Failed to read directory entry")?;
                    let path = entry.path();

                    if path
                        .extension()
                        .is_some_and(|ext| ext == "yaml" || ext == "yml")
                    {
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
                    // Return the parent directory name as content type
                    // In our structure: instance/module/content_type/file.yaml
                    return Ok(parent_str.to_string());
                }
            }
        }

        Err(anyhow::anyhow!(
            "Unable to infer content type from file path: {file_path}"
        ))
    }

    fn validate_object(&self, object: &XsiamObject) -> Result<()> {
        if object.id.is_empty() {
            return Err(anyhow::anyhow!("Object ID is required"));
        }

        // Name is now optional - some AppSec objects don't have names
        // Validation removed to support schema-compliant API responses

        if object.content_type.is_empty() {
            return Err(anyhow::anyhow!("Content type is required"));
        }

        // Content type validation removed - now module-aware via directory structure
        // The content_type comes from the directory path (instance/module/content_type/)
        // which is already validated by the module's content_types list

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ObjectMetadata, XsiamObject};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn object_with_content(content: BTreeMap<String, serde_json::Value>) -> XsiamObject {
        XsiamObject {
            id: "rule-1".to_string(),
            name: Some("Test rule".to_string()),
            description: "A rule".to_string(),
            content_type: "correlation_searches".to_string(),
            metadata: ObjectMetadata::default(),
            tenant_id: None,
            content,
        }
    }

    #[test]
    fn string_array_order_is_preserved() {
        // `suppression_fields` on a correlation rule and `mitre_technique_id_and_name`
        // on a BIOC are both arrays of strings whose order is part of the
        // configuration. Sorting them would mean the stored YAML no longer matches
        // the platform.
        let mut content = BTreeMap::new();
        content.insert(
            "suppression_fields".to_string(),
            json!(["zeta_field", "alpha_field", "middle_field"]),
        );

        let yaml = YamlParser::new()
            .serialize_object_deterministically(&object_with_content(content))
            .unwrap();

        let zeta = yaml.find("zeta_field").expect("zeta_field must be present");
        let alpha = yaml
            .find("alpha_field")
            .expect("alpha_field must be present");
        let middle = yaml
            .find("middle_field")
            .expect("middle_field must be present");
        assert!(
            zeta < alpha && alpha < middle,
            "array order was not preserved:\n{yaml}"
        );
    }

    #[test]
    fn nested_string_array_order_is_preserved() {
        let mut content = BTreeMap::new();
        content.insert(
            "config".to_string(),
            json!({"steps": ["third", "first", "second"]}),
        );

        let yaml = YamlParser::new()
            .serialize_object_deterministically(&object_with_content(content))
            .unwrap();

        let third = yaml.find("third").unwrap();
        let first = yaml.find("first").unwrap();
        assert!(
            third < first,
            "nested array order was not preserved:\n{yaml}"
        );
    }

    #[test]
    fn top_level_keys_are_sorted_for_stable_diffs() {
        let mut content = BTreeMap::new();
        content.insert("zebra".to_string(), json!(1));
        content.insert("apple".to_string(), json!(2));

        let yaml = YamlParser::new()
            .serialize_object_deterministically(&object_with_content(content))
            .unwrap();

        assert!(
            yaml.find("apple").unwrap() < yaml.find("zebra").unwrap(),
            "top-level keys should be alphabetical:\n{yaml}"
        );
    }

    #[test]
    fn serialisation_is_repeatable() {
        let mut content = BTreeMap::new();
        content.insert("b".to_string(), json!(["x", "y"]));
        content.insert("a".to_string(), json!({"nested": true}));
        let object = object_with_content(content);

        let parser = YamlParser::new();
        let first = parser.serialize_object_deterministically(&object).unwrap();
        let second = parser.serialize_object_deterministically(&object).unwrap();
        assert_eq!(first, second);
    }
}
