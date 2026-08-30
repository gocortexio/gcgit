// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::git_wrapper::GitWrapper;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

// Re-export ModuleConfig for public use
pub use crate::modules::ModuleConfig;

// Legacy XSIAM-only config for backwards compatibility
#[derive(Debug, Deserialize, Serialize)]
pub struct XsiamConfig {
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
    pub instance_name: String,
    /// Opt in to plain HTTP for this connection. Absent means HTTPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_http: Option<bool>,
}

// Multi-module configuration format (v2.0+)
#[derive(Debug, Deserialize, Serialize)]
pub struct ModulesConfig {
    /// Current name for the Cortex Platform module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<ModuleConfigData>,
    /// Name used before the rename. Still read so existing config files keep working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xsiam: Option<ModuleConfigData>,
    pub appsec: Option<ModuleConfigData>,
    pub agent: Option<ModuleConfigData>,
    pub cwp: Option<ModuleConfigData>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModuleConfigData {
    pub enabled: Option<bool>,
    pub fqdn: String,
    pub api_key: String,
    pub api_key_id: String,
    /// Opt in to plain HTTP for this connection. Absent means HTTPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_http: Option<bool>,
}

// Combined config file format supporting both legacy and multi-module
#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigFile {
    pub instance_name: String,

    // Legacy single-module format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xsiam: Option<XsiamConfig>,

    // New multi-module format (v2.0+)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<ModulesConfig>,
}

pub struct ConfigManager;

impl ConfigManager {
    pub fn new() -> Self {
        Self
    }

    // Load configuration for a specific module in an instance
    pub fn load_module_config(&self, instance_name: &str, module_id: &str) -> Result<ModuleConfig> {
        let config_path = format!("{instance_name}/config.toml");

        if !Path::new(&config_path).exists() {
            return Err(anyhow::anyhow!(
                "Instance '{instance_name}' not found. Run 'gcgit init --instance {instance_name}' first"
            ));
        }

        warn_if_world_readable(&config_path);

        let config_content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {config_path}"))?;

        let config: ConfigFile = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse config file: {config_path}"))?;

        // Try new multi-module format first
        if let Some(modules) = &config.modules {
            // "platform" was called "xsiam" before the rename. A config file written
            // by an earlier release has only the old section, so both are accepted and
            // the current name wins where both are present.
            let module_data = match module_id {
                "platform" | "xsiam" => modules.platform.as_ref().or(modules.xsiam.as_ref()),
                "appsec" => modules.appsec.as_ref(),
                "agent" => modules.agent.as_ref(),
                "cwp" => modules.cwp.as_ref(),
                _ => None,
            };

            if let Some(data) = module_data {
                let force_http = resolve_force_http(data.force_http, module_id);
                return Ok(ModuleConfig {
                    enabled: data.enabled.unwrap_or(true),
                    fqdn: resolve_with_fallback(&data.fqdn, "DEMISTO_BASE_URL", "fqdn", module_id)?,
                    api_key: resolve_with_fallback(
                        &data.api_key,
                        "DEMISTO_API_KEY",
                        "api_key",
                        module_id,
                    )?,
                    api_key_id: resolve_with_fallback(
                        &data.api_key_id,
                        "XSIAM_AUTH_ID",
                        "api_key_id",
                        module_id,
                    )?,
                    force_http,
                });
            }
        }

        // Fall back to the legacy single-module format, which only ever described
        // what is now the platform module.
        if module_id == "platform" || module_id == "xsiam" {
            if let Some(xsiam) = &config.xsiam {
                let force_http = resolve_force_http(xsiam.force_http, module_id);
                return Ok(ModuleConfig {
                    enabled: true,
                    fqdn: resolve_with_fallback(
                        &xsiam.fqdn,
                        "DEMISTO_BASE_URL",
                        "fqdn",
                        module_id,
                    )?,
                    api_key: resolve_with_fallback(
                        &xsiam.api_key,
                        "DEMISTO_API_KEY",
                        "api_key",
                        module_id,
                    )?,
                    api_key_id: resolve_with_fallback(
                        &xsiam.api_key_id,
                        "XSIAM_AUTH_ID",
                        "api_key_id",
                        module_id,
                    )?,
                    force_http,
                });
            }
        }

        Err(anyhow::anyhow!(
            "Module '{module_id}' not configured in instance '{instance_name}'"
        ))
    }

    pub fn create_test_config() -> Result<XsiamConfig> {
        let fqdn =
            std::env::var("XSIAM_FQDN").context("XSIAM_FQDN environment variable not set")?;
        let api_key =
            std::env::var("XSIAM_API_KEY").context("XSIAM_API_KEY environment variable not set")?;
        let api_key_id = std::env::var("XSIAM_API_KEY_ID")
            .context("XSIAM_API_KEY_ID environment variable not set")?;

        Ok(XsiamConfig {
            fqdn,
            api_key,
            api_key_id,
            instance_name: "test".to_string(),
            // No environment override: see resolve_force_http.
            force_http: None,
        })
    }

    /// Create a new instance directory, its module subdirectories, a config.toml
    /// template and a Git repository.
    ///
    /// Refuses to overwrite an existing config.toml unless `force` is set. The
    /// template is written with truncation, so without this check re-running init
    /// against a configured instance would silently discard working credentials.
    pub fn init_instance(&self, instance_name: &str, force: bool) -> Result<()> {
        let config_path = format!("{instance_name}/config.toml");
        if Path::new(&config_path).exists() && !force {
            return Err(anyhow::anyhow!(
                "Instance '{instance_name}' already exists and has a config file at {config_path}. \
                 Re-running init would overwrite it. Pass --force to replace the existing \
                 configuration, or choose a different instance name."
            ));
        }

        // Create instance directory
        fs::create_dir_all(instance_name)
            .with_context(|| format!("Failed to create instance directory: {instance_name}"))?;

        // Create module subdirectories using module registry
        let module_registry = crate::modules::ModuleRegistry::load();
        for module in module_registry.all_modules() {
            let module_path = format!("{}/{}", instance_name, module.id());
            fs::create_dir_all(&module_path)
                .with_context(|| format!("Failed to create module directory: {module_path}"))?;

            // Create content type subdirectories within each module
            for content_type in module.content_types() {
                let content_path = format!("{}/{}", module_path, content_type.name);
                fs::create_dir_all(&content_path).with_context(|| {
                    format!("Failed to create content type directory: {content_path}")
                })?;
            }
        }

        // Create config.toml template with multi-module format (v2.0+)
        let config_template = ConfigFile {
            instance_name: instance_name.to_string(),
            xsiam: None, // Use new modules format instead
            modules: Some(ModulesConfig {
                // New instances get the current section name. The legacy "xsiam"
                // section is still read, but never generated.
                xsiam: None,
                platform: Some(ModuleConfigData {
                    enabled: Some(true),
                    fqdn: "${XSIAM_FQDN}".to_string(),
                    api_key: "${XSIAM_API_KEY}".to_string(),
                    api_key_id: "${XSIAM_API_KEY_ID}".to_string(),
                    force_http: Some(false),
                }),
                appsec: Some(ModuleConfigData {
                    enabled: Some(true),
                    fqdn: "${XSIAM_FQDN}".to_string(), // Often same as XSIAM
                    api_key: "${XSIAM_API_KEY}".to_string(),
                    api_key_id: "${XSIAM_API_KEY_ID}".to_string(),
                    force_http: Some(false),
                }),
                agent: Some(ModuleConfigData {
                    enabled: Some(true),
                    fqdn: "${XSIAM_FQDN}".to_string(), // Same tenant as XSIAM
                    api_key: "${XSIAM_API_KEY}".to_string(),
                    api_key_id: "${XSIAM_API_KEY_ID}".to_string(),
                    force_http: Some(false),
                }),
                cwp: Some(ModuleConfigData {
                    enabled: Some(true),
                    fqdn: "${XSIAM_FQDN}".to_string(), // Same tenant as XSIAM
                    api_key: "${XSIAM_API_KEY}".to_string(),
                    api_key_id: "${XSIAM_API_KEY_ID}".to_string(),
                    force_http: Some(false),
                }),
            }),
        };

        let config_content = toml::to_string_pretty(&config_template)
            .context("Failed to serialize config template")?;

        write_restricted(&config_path, config_content.as_bytes())
            .with_context(|| format!("Failed to write config file: {config_path}"))?;

        // Prepare the Git repository that will hold this instance. If the instance
        // already sits inside a repository, that one is used and no nested repository
        // is created, which is what a continuous integration checkout needs.
        let git_repo = GitWrapper::new_for_instance(instance_name)
            .with_context(|| format!("Failed to prepare a git repository for: {instance_name}"))?;
        if git_repo.uses_enclosing_repository() {
            println!("Using the existing repository at {}", git_repo.location());
        }

        // Create .gitignore file to exclude config.toml from version control
        let gitignore_path = format!("{instance_name}/.gitignore");
        let gitignore_content = "*.toml\n";
        fs::write(&gitignore_path, gitignore_content)
            .with_context(|| format!("Failed to create .gitignore file: {gitignore_path}"))?;

        Ok(())
    }
}

/// Write `contents` to `path` with owner-only permissions (0600 on Unix).
/// Permissions are enforced both at creation time and via an explicit chmod
/// after writing, so pre-existing files with insecure modes are corrected.
/// On non-Unix platforms falls back to a plain write.
fn write_restricted(path: &str, contents: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::fs::{OpenOptions, Permissions};
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to open {path} for writing"))?;
        file.write_all(contents)
            .with_context(|| format!("Failed to write to {path}"))?;
        // Explicitly enforce 0600 to cover pre-existing files whose permissions
        // were not changed by mode() (mode() only applies at creation time).
        fs::set_permissions(path, Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {path}"))?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        fs::write(path, contents).with_context(|| format!("Failed to write {path}"))?;
        Ok(())
    }
}

/// Emit a warning if `path` is readable by group or others (Unix only).
fn warn_if_world_readable(path: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                eprintln!(
                    "[WARN] Config file '{}' has permissions {:04o} and is readable by group or \
                     others. To protect your API credentials run: chmod 600 {}",
                    path,
                    mode & 0o777,
                    path
                );
            }
        }
    }
}

/// Substitute every `${VAR}` reference in `input`.
///
/// Handles references embedded in a larger string, such as
/// `"${TENANT}.xdr.au.paloaltonetworks.com"`. Previously only a value that was
/// entirely one reference was substituted, so an embedded reference was passed
/// through literally and used as a hostname.
///
/// A reference to a variable that is unset or empty makes the whole value resolve
/// to empty, which is what triggers the fallback chain in `resolve_with_fallback`.
/// The names of any such variables are returned so the caller can say which ones
/// were missing.
fn expand_env_vars(input: &str) -> (String, Vec<String>) {
    let mut output = String::with_capacity(input.len());
    let mut missing = Vec::new();
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        let Some(end_offset) = rest[start..].find('}') else {
            // No closing brace: the remainder is literal text.
            break;
        };
        let end = start + end_offset;

        output.push_str(&rest[..start]);
        let var_name = &rest[start + 2..end];

        match env::var(var_name) {
            Ok(value) if !value.is_empty() => output.push_str(&value),
            _ => missing.push(var_name.to_string()),
        }

        rest = &rest[end + 1..];
    }

    output.push_str(rest);

    if missing.is_empty() {
        (output, missing)
    } else {
        // A partially substituted value would be wrong in a way that is hard to
        // diagnose, so treat any missing reference as "no value".
        (String::new(), missing)
    }
}

/// Decide whether this connection sends requests over plain HTTP.
///
/// The config file is the only source. There is deliberately no environment
/// variable: an ambient variable would downgrade the transport for every config
/// file that predates this option, which is all of them, and reqwest applies
/// HTTP_PROXY to http:// requests, so an environment-only actor could route
/// credential-bearing requests through a proxy without modifying any file and
/// without leaving a trace in Git. Requiring a config file edit keeps the
/// capability with whoever can already read the API key out of that same file.
pub fn resolve_force_http(configured: Option<bool>, module_id: &str) -> bool {
    let force_http = configured.unwrap_or(false);

    if force_http {
        eprintln!(
            "[WARN] Module '{module_id}' is configured with force_http, so requests are sent \
             over plain HTTP. The API key and key ID travel unencrypted and can be read by \
             anything on the network path. Use this only against a local or trusted \
             development endpoint, never against a production tenant."
        );
    }

    force_http
}

/// Strip a scheme, credentials, trailing slash and any path from a configured FQDN.
///
/// Applied to every source of the value, not just the fallback variable. A value
/// arriving as `https://api-gocortex.xdr.au.paloaltonetworks.com` previously reached
/// URL construction verbatim and produced `https://https://api-tenant...`.
///
/// Userinfo is discarded rather than passed through: a value of the form
/// `id:secret@host` would otherwise place the secret into every constructed URL and
/// therefore into every error message. Note that the scheme is only stripped here,
/// never interpreted; plain HTTP is selected by the force_http option alone.
fn normalise_fqdn(value: &str) -> String {
    let host = value
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');

    // Drop anything after the authority so a pasted URL with a path still works.
    let authority = match host.find('/') {
        Some(index) => &host[..index],
        None => host,
    };

    // Drop any userinfo prefix so credentials never reach a URL.
    match authority.rfind('@') {
        Some(index) => authority[index + 1..].to_string(),
        None => authority.to_string(),
    }
}

fn resolve_with_fallback(
    value: &str,
    fallback_var: &str,
    field_label: &str,
    module_id: &str,
) -> Result<String> {
    let (expanded, missing) = expand_env_vars(value);

    if !expanded.is_empty() {
        return Ok(finalise_field(expanded, field_label));
    }

    match env::var(fallback_var) {
        Ok(val) if !val.is_empty() => {
            eprintln!(
                "[INFO] Using {fallback_var} as fallback for {field_label} (module: {module_id})"
            );
            Ok(finalise_field(val, field_label))
        }
        _ => {
            let cause = if missing.is_empty() {
                format!("Configuration field '{field_label}' is empty")
            } else {
                format!(
                    "Configuration field '{field_label}' references environment variable(s) {} which are not set",
                    missing.join(", ")
                )
            };
            Err(anyhow::anyhow!(
                "{cause} and fallback variable {fallback_var} is not set (module: {module_id})"
            ))
        }
    }
}

/// Apply per-field normalisation once a value has been resolved from any source.
fn finalise_field(value: String, field_label: &str) -> String {
    if field_label == "fqdn" {
        normalise_fqdn(&value)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fqdn_normalisation_strips_scheme_path_and_trailing_slash() {
        assert_eq!(
            normalise_fqdn("api-gocortex.xdr.au.paloaltonetworks.com"),
            "api-gocortex.xdr.au.paloaltonetworks.com"
        );
        assert_eq!(
            normalise_fqdn("https://api-gocortex.xdr.au.paloaltonetworks.com"),
            "api-gocortex.xdr.au.paloaltonetworks.com"
        );
        assert_eq!(
            normalise_fqdn("http://api-gocortex.xdr.au.paloaltonetworks.com/"),
            "api-gocortex.xdr.au.paloaltonetworks.com"
        );
        assert_eq!(
            normalise_fqdn("https://api-gocortex.xdr.au.paloaltonetworks.com/public_api/v1"),
            "api-gocortex.xdr.au.paloaltonetworks.com"
        );
        assert_eq!(normalise_fqdn("  api-t.example.com  "), "api-t.example.com");
    }

    #[test]
    fn env_expansion_handles_embedded_references() {
        // SAFETY: single-threaded test setting a variable it alone reads.
        unsafe { env::set_var("GCGIT_TEST_TENANT", "api-gocortex") };

        let (value, missing) = expand_env_vars("${GCGIT_TEST_TENANT}.xdr.au.paloaltonetworks.com");
        assert_eq!(value, "api-gocortex.xdr.au.paloaltonetworks.com");
        assert!(missing.is_empty());

        let (whole, missing) = expand_env_vars("${GCGIT_TEST_TENANT}");
        assert_eq!(whole, "api-gocortex");
        assert!(missing.is_empty());

        unsafe { env::remove_var("GCGIT_TEST_TENANT") };
    }

    #[test]
    fn env_expansion_reports_missing_variables() {
        let (value, missing) = expand_env_vars("${GCGIT_TEST_DEFINITELY_UNSET}.example.com");
        assert!(
            value.is_empty(),
            "a missing variable must not yield a partial value"
        );
        assert_eq!(missing, vec!["GCGIT_TEST_DEFINITELY_UNSET".to_string()]);
    }

    #[test]
    fn literal_values_pass_through_unchanged() {
        let (value, missing) = expand_env_vars("api-gocortex.xdr.au.paloaltonetworks.com");
        assert_eq!(value, "api-gocortex.xdr.au.paloaltonetworks.com");
        assert!(missing.is_empty());
    }

    #[test]
    fn force_http_defaults_to_off() {
        // Every config.toml written before this option existed omits it, and those
        // connections must keep using HTTPS.
        assert!(!resolve_force_http(None, "xsiam"));
    }

    #[test]
    fn force_http_honours_an_explicit_setting() {
        assert!(resolve_force_http(Some(true), "xsiam"));
        assert!(!resolve_force_http(Some(false), "xsiam"));
    }

    #[test]
    fn a_config_using_the_previous_module_name_still_resolves() {
        // Written by a release before the rename. Credentials must still resolve, or
        // the upgrade silently breaks every existing instance.
        let toml_text = r#"
instance_name = "prod"

[modules.xsiam]
enabled = true
fqdn = "api-gocortex.xdr.au.paloaltonetworks.com"
api_key = "k"
api_key_id = "1"
"#;
        let parsed: ConfigFile = toml::from_str(toml_text).expect("legacy section must parse");
        let modules = parsed.modules.unwrap();
        assert!(modules.xsiam.is_some());
        assert!(modules.platform.is_none());
    }

    #[test]
    fn the_current_module_name_parses() {
        let toml_text = r#"
instance_name = "prod"

[modules.platform]
enabled = true
fqdn = "api-gocortex.xdr.au.paloaltonetworks.com"
api_key = "k"
api_key_id = "1"
"#;
        let parsed: ConfigFile = toml::from_str(toml_text).expect("current section must parse");
        let modules = parsed.modules.unwrap();
        assert!(modules.platform.is_some());
        assert!(modules.xsiam.is_none());
    }

    #[test]
    fn a_config_without_force_http_still_parses() {
        // Backwards compatibility: the field is absent from every existing file.
        let toml_text = r#"
instance_name = "prod"

[modules.xsiam]
enabled = true
fqdn = "api-gocortex.xdr.au.paloaltonetworks.com"
api_key = "k"
api_key_id = "1"
"#;
        let parsed: ConfigFile = toml::from_str(toml_text).expect("legacy config must still parse");
        let xsiam = parsed.modules.unwrap().xsiam.unwrap();
        assert_eq!(xsiam.force_http, None);
    }

    #[test]
    fn a_config_with_force_http_parses() {
        let toml_text = r#"
instance_name = "dev"

[modules.xsiam]
enabled = true
fqdn = "localhost:8080"
api_key = "k"
api_key_id = "1"
force_http = true
"#;
        let parsed: ConfigFile =
            toml::from_str(toml_text).expect("config with force_http must parse");
        let xsiam = parsed.modules.unwrap().xsiam.unwrap();
        assert_eq!(xsiam.force_http, Some(true));
    }

    #[test]
    fn normalise_fqdn_discards_userinfo() {
        // Credentials embedded in the host would otherwise appear in every URL and
        // every error message.
        assert_eq!(
            normalise_fqdn("https://id:secret@api-t.example.com"),
            "api-t.example.com"
        );
        assert_eq!(
            normalise_fqdn("user@api-t.example.com"),
            "api-t.example.com"
        );
    }

    #[test]
    fn a_scheme_in_fqdn_does_not_select_plain_http() {
        // Only force_http chooses the scheme. Stripping http:// here must not be
        // mistaken for opting in to it.
        assert_eq!(normalise_fqdn("http://localhost:8080"), "localhost:8080");
        assert!(!resolve_force_http(None, "xsiam"));
    }

    #[test]
    fn normalise_fqdn_keeps_a_port() {
        // A local mock is typically reached on an explicit port.
        assert_eq!(normalise_fqdn("http://localhost:8080"), "localhost:8080");
        assert_eq!(normalise_fqdn("localhost:8080"), "localhost:8080");
    }

    #[test]
    fn unterminated_reference_is_treated_as_literal_text() {
        let (value, missing) = expand_env_vars("${UNCLOSED");
        assert_eq!(value, "${UNCLOSED");
        assert!(missing.is_empty());
    }
}
