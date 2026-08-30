// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};

mod api;
mod cli;
mod config;
mod git_wrapper;
mod lock;
mod modules;
mod parser;
mod types;

use cli::{Cli, Commands, ModuleCommands};
use config::ConfigManager;
use git_wrapper::GitWrapper;
use lock::InstanceLock;
use modules::ModuleRegistry;
use parser::YamlParser;

/// Sanitise a remote-supplied ID so it is safe to embed in a local filename.
/// Keeps only alphanumeric characters, hyphens, and underscores; everything
/// else (including `/`, `\`, `.`, null bytes, and `..` sequences) is replaced
/// with `_`.  This prevents path-traversal attacks when object IDs from an
/// untrusted API response are used to construct file paths.
fn sanitize_id_for_filename(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Confirm an instance exists before any work is attempted against it.
///
/// Without this the first thing to fail is lock acquisition, which reports that a lock
/// file could not be created in a directory that does not exist. That names an internal
/// detail rather than the actual problem, and is especially unhelpful when the instance
/// is the implicit "default" because --instance was not given.
fn require_instance(instance_name: &str, explicit: bool) -> Result<()> {
    let dir = std::path::Path::new(instance_name);
    if dir.join("config.toml").is_file() {
        return Ok(());
    }

    if dir.is_dir() {
        return Err(anyhow::anyhow!(
            "Instance '{instance_name}' has no config.toml. \
             Run 'gcgit init --instance {instance_name}' to create one."
        ));
    }

    if explicit {
        Err(anyhow::anyhow!(
            "Instance '{instance_name}' does not exist. \
             Run 'gcgit init --instance {instance_name}' first."
        ))
    } else {
        // No --instance was given, so the name came from the default.
        Err(anyhow::anyhow!(
            "No instance given and no '{instance_name}' instance exists. \
             Pass --instance NAME, or run 'gcgit init --instance {instance_name}' to create the default."
        ))
    }
}

/// A content type named on the command line, optionally qualified by module.
///
/// `policies` matches that content type in whichever module is being pulled.
/// `cwp/policies` matches it only in CWP, which matters because AppSec has a content
/// type of the same name. The separator is the one the repository already uses between
/// a module and its content types, so a path copied out of the working tree is a valid
/// argument.
struct ContentTypeSelector {
    module: Option<String>,
    content_type: String,
}

impl ContentTypeSelector {
    fn parse(raw: &str) -> Self {
        match raw.split_once('/') {
            Some((module, content_type)) => Self {
                module: Some(module.trim().to_string()),
                content_type: content_type.trim().to_string(),
            },
            None => Self {
                module: None,
                content_type: raw.trim().to_string(),
            },
        }
    }

    /// Whether this selector applies to a content type in the module being pulled.
    fn matches(&self, module_id: &str, content_type: &str) -> bool {
        if self.content_type != content_type {
            return false;
        }
        match &self.module {
            Some(module) => module == module_id,
            None => true,
        }
    }
}

/// Directory name to use for a module's files inside an instance.
///
/// A module that has been renamed keeps writing to its previous directory wherever one
/// already exists, so an existing backup does not fragment across two directory names
/// or appear to lose everything it had. New instances use the current name.
fn module_dir_name(instance_name: &str, module: &dyn modules::Module) -> String {
    if let Some(legacy) = module.legacy_id() {
        if std::path::Path::new(instance_name).join(legacy).is_dir() {
            return legacy.to_string();
        }
    }
    module.id().to_string()
}

/// Build a filename suffix that distinguishes two objects sharing a name.
///
/// The sanitised ID alone is not enough: sanitize_id_for_filename maps every
/// character outside [A-Za-z0-9_-] to an underscore, so it is many-to-one and two
/// different IDs can produce the same suffix, silently overwriting one object. A
/// short hash of the untouched ID is appended so the suffix is injective in
/// practice while staying readable.
fn disambiguator(id: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{}_{:08x}", sanitize_id_for_filename(id), hash as u32)
}

/// Turn a remote-supplied object name into a safe filename stem.
///
/// Replaces path separators and any control character, then trims. A live tenant
/// returned an XQL library entry whose name ended in two newline characters, which
/// produced a file whose name contained literal newlines: legal on disk, but it
/// breaks shell pipelines, `ls` output and anything parsing Git porcelain.
///
/// Returns None when nothing usable remains, so the caller can fall back to the
/// object ID.
fn sanitize_name_for_filename(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            // Path separators, shell-hostile punctuation, characters Windows
            // reserves, spaces, and any control character all become underscores.
            // is_control covers only the Cc category. Format characters (Cf) such as
            // U+202E RIGHT-TO-LEFT OVERRIDE and U+200B ZERO WIDTH SPACE are invisible
            // in a terminal and can make one filename display as another, so they are
            // replaced too.
            let is_format_char = matches!(c as u32,
                0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x08E2
                | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064
                | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB);
            if c.is_control()
                || is_format_char
                || c == ' '
                || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                c
            }
        })
        .collect();

    // Trailing dots and underscores come from trimmed control characters and from
    // names that ended in whitespace; both make for confusing filenames.
    let trimmed = cleaned.trim_matches(|c: char| c == '_' || c == '.' || c.is_whitespace());

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Delete YAML files in `dir` that the current pull did not write, returning the
/// filenames removed.
///
/// Called only after a content type has pulled successfully. An object deleted on
/// the platform simply stops appearing in the response, so without this the local
/// file and its Git history would continue to assert a configuration that no longer
/// exists. `pull_content_type` reports a structural mismatch as an error rather than
/// an empty result, so a malformed response cannot reach this function and delete
/// everything.
fn prune_stale_files(dir: &str, keep: &std::collections::HashSet<String>) -> Result<Vec<String>> {
    let stale = stale_files(dir, keep)?;
    for file_name in &stale {
        let path = std::path::Path::new(dir).join(file_name);
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove stale file: {}", path.display()))?;
    }
    Ok(stale)
}

/// List the YAML files in `dir` that the current pull did not write.
fn stale_files(dir: &str, keep: &std::collections::HashSet<String>) -> Result<Vec<String>> {
    let mut removed = Vec::new();

    let dir_path = std::path::Path::new(dir);
    if !dir_path.exists() {
        return Ok(removed);
    }

    let entries =
        std::fs::read_dir(dir_path).with_context(|| format!("Failed to read directory: {dir}"))?;

    for entry in entries {
        let entry = entry.with_context(|| format!("Failed to read entry in {dir}"))?;
        let file_path = entry.path();

        if !file_path.is_file() {
            continue;
        }
        if !file_path
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            continue;
        }

        let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if keep.contains(file_name) {
            continue;
        }

        removed.push(file_name.to_string());
    }

    removed.sort();
    Ok(removed)
}

/// Split a comma-separated module list in the command position.
///
/// Returns None unless the first argument is a list, so a single-module invocation
/// takes the ordinary path and behaves exactly as before.
fn requested_module_list(args: &[String]) -> Option<Vec<String>> {
    let candidate = args.get(1)?;
    if candidate.starts_with('-') || !candidate.contains(',') {
        return None;
    }

    let mut modules = Vec::new();
    for name in candidate.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // Preserve the order given, ignoring a repeat.
        if !modules.iter().any(|held: &String| held == name) {
            modules.push(name.to_string());
        }
    }

    if modules.is_empty() {
        None
    } else {
        Some(modules)
    }
}

/// Run one subcommand against several modules in turn.
///
/// A module that fails does not stop the ones after it: a scheduled backup should
/// capture what it can rather than abandoning everything because one endpoint was
/// unavailable. The summary names any that failed, and --strict makes that a non-zero
/// exit, matching how a single pull already treats a failing content type.
async fn run_across_modules(modules: Vec<String>, args: &[String]) -> Result<()> {
    let registry = ModuleRegistry::load();

    // Validate every name before doing any work, so a typo does not surface halfway
    // through after some modules have already been written.
    for name in &modules {
        if registry.get(name).is_none() {
            let mut known: Vec<&str> = registry.all_modules().iter().map(|m| m.id()).collect();
            known.sort_unstable();
            return Err(anyhow::anyhow!(
                "Unknown module '{name}'. Available: {}",
                known.join(", ")
            ));
        }
    }

    let strict = args.iter().any(|a| a == "--strict");
    let mut failed: Vec<String> = Vec::new();

    for name in &modules {
        println!("=== {name} ===");

        // Re-parse the original invocation with this single module in the command
        // position, so every flag is interpreted exactly as clap would normally.
        let mut single: Vec<String> = Vec::with_capacity(args.len());
        single.push(args[0].clone());
        single.push(name.clone());
        single.extend_from_slice(&args[2..]);

        let cli = match Cli::try_parse_from(&single) {
            Ok(cli) => cli,
            Err(e) => {
                e.print().ok();
                return Err(anyhow::anyhow!(
                    "Could not interpret the command for module '{name}'"
                ));
            }
        };

        let outcome = match cli.command {
            Some(Commands::Platform { command }) => {
                handle_module_command("platform", command).await
            }
            Some(Commands::Appsec { command }) => handle_module_command("appsec", command).await,
            Some(Commands::Agent { command }) => handle_module_command("agent", command).await,
            Some(Commands::Cwp { command }) => handle_module_command("cwp", command).await,
            _ => Err(anyhow::anyhow!(
                "A module list can only be used with a module command such as pull, diff or test"
            )),
        };

        if let Err(e) = outcome {
            println!("[ERROR] {name} failed - {e}");
            failed.push(name.clone());
        }
        println!();
    }

    let completed = modules.len() - failed.len();
    println!("{completed} of {} module(s) completed.", modules.len());
    if !failed.is_empty() {
        println!("Failed: {}", failed.join(", "));
        if strict {
            return Err(anyhow::anyhow!(
                "{} module(s) failed: {}",
                failed.len(),
                failed.join(", ")
            ));
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(modules) = requested_module_list(&args) {
        return run_across_modules(modules, &args).await;
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Platform { command }) => {
            handle_module_command("platform", command).await?;
        }
        Some(Commands::Appsec { command }) => {
            handle_module_command("appsec", command).await?;
        }
        Some(Commands::Agent { command }) => {
            handle_module_command("agent", command).await?;
        }
        Some(Commands::Cwp { command }) => {
            handle_module_command("cwp", command).await?;
        }
        Some(Commands::Init { instance, force }) => {
            handle_init_command(instance, force).await?;
        }
        Some(Commands::Status { instance }) => {
            handle_status_command(instance).await?;
        }
        Some(Commands::Deploy {
            instance: _,
            message: _,
            files: _,
        }) => {
            eprintln!("[ERROR] Feature not yet available");
            eprintln!();
            eprintln!("Usage: gcgit deploy [OPTIONS]");
            eprintln!();
            eprintln!("This feature is still under development.");
            eprintln!("Visit https://gocortex.io for updates on feature availability.");
            std::process::exit(1);
        }
        Some(Commands::Validate { instance, files }) => {
            handle_validate_command(instance, files).await?;
        }
        None => {
            // No command provided, show help with version (same as --help)
            let mut cmd = Cli::command();
            cmd.print_long_help().unwrap();
            std::process::exit(0);
        }
    }

    Ok(())
}

async fn handle_module_command(module_id: &str, command: ModuleCommands) -> Result<()> {
    // Get the module from registry
    let module_registry = ModuleRegistry::load();
    let module = module_registry
        .get(module_id)
        .ok_or_else(|| anyhow::anyhow!("Module '{module_id}' not found"))?;

    match command {
        ModuleCommands::Push { instance: _ } => {
            let module_upper = module_id.to_uppercase();
            eprintln!("[ERROR] Feature not yet available");
            eprintln!();
            eprintln!("Usage: gcgit {module_id} push --instance <NAME>");
            eprintln!();
            eprintln!("Push operations for {module_upper} are still under development.");
            eprintln!("Visit https://gocortex.io for updates on feature availability.");
            std::process::exit(1);
        }
        ModuleCommands::Pull {
            instance,
            strict,
            content_type,
            skip,
            dry_run,
            no_git,
            quiet,
        } => {
            let explicit = instance.is_some();
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            require_instance(&instance_name, explicit)?;

            // Acquire lock to prevent concurrent operations on the same instance
            let _lock = InstanceLock::acquire(&instance_name)?;

            let config_manager = ConfigManager::new();
            let module_config = config_manager.load_module_config(&instance_name, module_id)?;

            // Check if module is enabled
            if !module_config.enabled {
                println!("Module '{module_id}' is disabled in instance '{instance_name}'. Enable it in config.toml to use this command.");
                return Ok(());
            }

            let module_client = api::ModuleClient::new(module_config, module.base_api_path());
            let yaml_parser = YamlParser::new();

            // Where this module's files live. An instance created before the rename
            // keeps its original directory.
            let module_dir = module_dir_name(&instance_name, module);
            if module_dir != module.id() {
                println!("Using the existing '{module_dir}' directory for this module");
            }

            // Pull each content type defined in the module, or only those named.
            let all_content_types = module.content_types();
            if let Some(unknown) = content_type
                .iter()
                .find(|name| !all_content_types.iter().any(|ct| ct.name == name.as_str()))
            {
                let known: Vec<&str> = all_content_types.iter().map(|ct| ct.name).collect();
                return Err(anyhow::anyhow!(
                    "Module '{module_id}' has no content type '{unknown}'. Available: {}",
                    known.join(", ")
                ));
            }
            // A skipped name this module does not have is ignored rather than
            // rejected, so one list works across a run covering several modules. A name
            // that matches nothing anywhere is caught before any work starts.
            // A trailing comma, or a stray one between spaced values, splits into an
            // empty string. Drop those instead of treating them as a name.
            let skip: Vec<String> = skip
                .into_iter()
                .map(|raw| raw.trim().trim_matches(',').trim().to_string())
                .filter(|raw| !raw.is_empty())
                .collect();
            let content_type: Vec<String> = content_type
                .into_iter()
                .map(|raw| raw.trim().trim_matches(',').trim().to_string())
                .filter(|raw| !raw.is_empty())
                .collect();

            let selectors: Vec<ContentTypeSelector> = skip
                .iter()
                .map(|raw| ContentTypeSelector::parse(raw))
                .collect();
            if !selectors.is_empty() {
                let registry = ModuleRegistry::load();
                for (raw, selector) in skip.iter().zip(&selectors) {
                    if let Some(module) = &selector.module {
                        let Some(target) = registry.get(module) else {
                            let mut names: Vec<&str> =
                                registry.all_modules().iter().map(|m| m.id()).collect();
                            names.sort_unstable();
                            return Err(anyhow::anyhow!(
                                "'{raw}' names module '{module}', which does not exist. Available: {}",
                                names.join(", ")
                            ));
                        };
                        if !target
                            .content_types()
                            .iter()
                            .any(|ct| ct.name == selector.content_type)
                        {
                            let names: Vec<&str> =
                                target.content_types().iter().map(|ct| ct.name).collect();
                            return Err(anyhow::anyhow!(
                                "Module '{module}' has no content type '{}'. Available: {}",
                                selector.content_type,
                                names.join(", ")
                            ));
                        }
                        continue;
                    }

                    let known = registry
                        .all_modules()
                        .iter()
                        .flat_map(|m| m.content_types())
                        .any(|ct| ct.name == selector.content_type);
                    if !known {
                        let mut all: Vec<&str> = registry
                            .all_modules()
                            .iter()
                            .flat_map(|m| m.content_types())
                            .map(|ct| ct.name)
                            .collect();
                        all.sort_unstable();
                        all.dedup();
                        return Err(anyhow::anyhow!(
                            "No module has a content type '{raw}'. Available: {}",
                            all.join(", ")
                        ));
                    }
                }
            }

            let content_types: Vec<_> = all_content_types
                .iter()
                .filter(|ct| content_type.is_empty() || content_type.iter().any(|n| n == ct.name))
                .filter(|ct| !selectors.iter().any(|s| s.matches(module_id, ct.name)))
                .cloned()
                .collect();

            let skipped: Vec<&str> = all_content_types
                .iter()
                .filter(|ct| selectors.iter().any(|s| s.matches(module_id, ct.name)))
                .map(|ct| ct.name)
                .collect();
            if !skipped.is_empty() {
                println!("Skipping: {}", skipped.join(", "));
            }

            if content_types.is_empty() {
                println!("Nothing to pull: every content type in this module was excluded.");
                return Ok(());
            }

            if dry_run {
                println!("Dry run: no files will be written and nothing will be committed.\n");
            }

            let mut _total_pulled = 0;
            let mut pulled_files = Vec::new();
            let mut removed_files = Vec::new();
            let mut failed_content_types: Vec<&str> = Vec::new();

            let selected_count = content_types.len();

            for content_def in content_types {
                println!("Pulling {}...", content_def.name);
                match module_client.pull_content_type(&content_def).await {
                    Ok(outcome) => {
                        let objects = outcome.objects;
                        println!("  Found {} {}(s)", objects.len(), content_def.name);

                        // Filenames written for this content type during this pull.
                        // Anything else left in the directory is stale and is pruned below.
                        let mut written_filenames: std::collections::HashSet<String> =
                            std::collections::HashSet::new();

                        // Objects the confinement check refused to write. They are
                        // absent from written_filenames but their local files are not
                        // stale, so pruning must not run.
                        let mut skipped_for_safety = 0usize;

                        // Build base filenames and detect collisions
                        let base_names: Vec<String> = objects
                            .iter()
                            .map(|obj| {
                                obj.name
                                    .as_deref()
                                    .and_then(sanitize_name_for_filename)
                                    .unwrap_or_else(|| {
                                        format!(
                                            "{}_id_{}",
                                            content_def.name.trim_end_matches('s'),
                                            sanitize_id_for_filename(&obj.id)
                                        )
                                    })
                            })
                            .collect();

                        // Count occurrences of each base name, case-insensitively.
                        // macOS and Windows filesystems are case-insensitive by
                        // default, so two objects whose names differ only in case
                        // resolve to one file and the second silently overwrites the
                        // first. A live tenant returned exactly this: "Oracle
                        // credentials detected in code" alongside "Oracle Credentials
                        // detected in code". gcgit publishes macOS binaries, so this
                        // is data loss on a supported platform.
                        let mut name_counts: std::collections::HashMap<String, usize> =
                            std::collections::HashMap::new();
                        for name in &base_names {
                            *name_counts.entry(name.to_lowercase()).or_insert(0) += 1;
                        }

                        // Compute the expected base directory for path-confinement checks.
                        // We canonicalize the current working directory and append the
                        // instance/module/content_type prefix so we can verify that each
                        // constructed path remains inside it.
                        let expected_base = std::env::current_dir()
                            .map(|cwd| {
                                cwd.join(&instance_name)
                                    .join(&module_dir)
                                    .join(content_def.name)
                            })
                            .ok();

                        for (object, base_name) in objects.iter().zip(base_names.iter()) {
                            // Disambiguate colliding names by appending the sanitised object ID
                            let filename = if name_counts
                                .get(&base_name.to_lowercase())
                                .copied()
                                .unwrap_or(1)
                                > 1
                            {
                                format!("{}_{}", base_name, disambiguator(&object.id))
                            } else {
                                base_name.clone()
                            };

                            let file_path = format!(
                                "{}/{}/{}/{}.yaml",
                                instance_name, module_dir, content_def.name, filename
                            );

                            // Verify the path stays within the intended directory.
                            // We use std::path::Path::components() to check for `..` and
                            // absolute-path components without requiring the path to exist yet.
                            if let Some(ref base) = expected_base {
                                use std::path::Component;
                                let constructed = std::path::Path::new(&file_path);
                                let has_traversal = constructed.components().any(|c| {
                                    matches!(
                                        c,
                                        Component::ParentDir
                                            | Component::RootDir
                                            | Component::Prefix(_)
                                    )
                                });
                                if has_traversal {
                                    eprintln!("  SECURITY: Skipping object '{}' - constructed path '{}' attempts to escape the pull directory.", object.id, file_path);
                                    skipped_for_safety += 1;
                                    continue;
                                }
                                // Additional check: normalised absolute path must be inside base.
                                let absolute = std::env::current_dir()
                                    .unwrap_or_default()
                                    .join(constructed);
                                // Normalise by resolving `.` components without hitting the FS.
                                let mut normalised = std::path::PathBuf::new();
                                for component in absolute.components() {
                                    match component {
                                        Component::ParentDir => {
                                            normalised.pop();
                                        }
                                        Component::CurDir => {}
                                        other => normalised.push(other),
                                    }
                                }
                                if !normalised.starts_with(base) {
                                    eprintln!("  SECURITY: Skipping object '{}' - path '{}' resolves outside the pull directory.", object.id, file_path);
                                    skipped_for_safety += 1;
                                    continue;
                                }
                            }

                            if dry_run {
                                if !quiet {
                                    let status = if std::path::Path::new(&file_path).exists() {
                                        "would update"
                                    } else {
                                        "would create"
                                    };
                                    println!("  {status}: {file_path}");
                                }
                            } else {
                                yaml_parser.write_file(&file_path, object)?;
                                if !quiet {
                                    println!("  Pulled: {file_path}");
                                }
                            }
                            written_filenames.insert(format!("{filename}.yaml"));
                            let relative_path =
                                format!("{}/{}/{}.yaml", module_dir, content_def.name, filename);
                            pulled_files.push(relative_path);
                            _total_pulled += 1;
                        }

                        // Remove local files for objects that no longer exist remotely.
                        // Only ever from a complete pull: a partial one is missing
                        // objects that are still on the platform.
                        let content_dir =
                            format!("{}/{}/{}", instance_name, module_dir, content_def.name);
                        if !outcome.complete {
                            println!("  Skipping removal of stale files: this pull was incomplete");
                            continue;
                        }
                        if skipped_for_safety > 0 {
                            println!("  Skipping removal of stale files: {skipped_for_safety} object(s) were not written");
                            continue;
                        }
                        if dry_run {
                            match stale_files(&content_dir, &written_filenames) {
                                Ok(stale) => {
                                    // A removal is worth reporting even when quiet,
                                    // because it is the only destructive thing a pull
                                    // does. Quiet reduces it to a count rather than
                                    // hiding it.
                                    if quiet {
                                        if !stale.is_empty() {
                                            println!(
                                                "  Would remove {} stale file(s)",
                                                stale.len()
                                            );
                                        }
                                    } else {
                                        for file_name in stale {
                                            println!("  would remove: {content_dir}/{file_name} (no longer present on the platform)");
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!("  [WARN] Could not inspect {content_dir} - {e}")
                                }
                            }
                            continue;
                        }
                        match prune_stale_files(&content_dir, &written_filenames) {
                            Ok(stale) => {
                                if quiet && !stale.is_empty() {
                                    println!("  Removed {} stale file(s)", stale.len());
                                }
                                for file_name in stale {
                                    if !quiet {
                                        println!("  Removed: {content_dir}/{file_name} (no longer present on the platform)");
                                    }
                                    removed_files.push(format!(
                                        "{}/{}/{}",
                                        module_dir, content_def.name, file_name
                                    ));
                                }
                            }
                            Err(e) => {
                                println!(
                                    "  [ERROR] Failed to remove stale files in {content_dir} - {e}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        failed_content_types.push(content_def.name);
                        println!("  [ERROR] Failed to pull {} - {}", content_def.name, e);
                        println!("  (Local files for this content type are left untouched.)");
                    }
                }
            }

            if !failed_content_types.is_empty() {
                println!(
                    "\n[ERROR] {} of {} content types failed to pull: {}",
                    failed_content_types.len(),
                    selected_count,
                    failed_content_types.join(", ")
                );
                println!("Local files for those content types were left unchanged.");
            }

            // Auto-commit pulled changes using Git's native change detection
            if dry_run {
                println!(
                    "\nDry run complete: {} file(s) would be written.",
                    pulled_files.len()
                );
            } else if no_git {
                println!(
                    "\nWrote {} file(s); {} removed. Staging and committing skipped (--no-git).",
                    pulled_files.len(),
                    removed_files.len()
                );
            } else if !pulled_files.is_empty() || !removed_files.is_empty() {
                println!("\nProcessing pulled files for Git repository...");

                match GitWrapper::new_for_instance(&instance_name) {
                    Ok(git_wrapper) => {
                        if git_wrapper.uses_enclosing_repository() {
                            println!(
                                "Using the existing repository at {}",
                                git_wrapper.location()
                            );
                        }
                        // Use Git's native change detection - much faster than API calls
                        match git_wrapper.has_changes_after_add(&pulled_files, &removed_files) {
                            Ok((true, changed_count, changed_files)) => {
                                // Create descriptive commit message with changed files
                                let changed_file_names: Vec<String> = changed_files
                                    .iter()
                                    .map(|path| {
                                        // Extract just the filename from the path for readability
                                        if let Some(filename) = path.split('/').next_back() {
                                            filename.replace(".yaml", "")
                                        } else {
                                            path.clone()
                                        }
                                    })
                                    .collect();

                                let module_upper = module_id.to_uppercase();
                                let commit_message = if changed_count == 1 {
                                    format!(
                                        "Auto-commit: Updated {} from {}",
                                        changed_file_names[0], module_upper
                                    )
                                } else if changed_count <= 3 {
                                    format!(
                                        "Auto-commit: Updated {} from {}",
                                        changed_file_names.join(", "),
                                        module_upper
                                    )
                                } else {
                                    format!(
                                        "Auto-commit: Updated {} files from {} ({})",
                                        changed_count,
                                        module_upper,
                                        changed_file_names[..2].join(", ")
                                    )
                                };

                                if let Err(e) = git_wrapper.commit(&commit_message) {
                                    println!("Warning: Failed to commit changes: {e}");
                                } else {
                                    let file_word =
                                        if changed_count == 1 { "file" } else { "files" };
                                    println!("Successfully processed {} pulled files to instance Git repository", pulled_files.len());
                                    println!("  {changed_count} {file_word} actually changed and committed");
                                }
                            }
                            Ok((false, _, _)) => {
                                println!("Successfully processed {} pulled files to instance Git repository", pulled_files.len());
                                println!("  No Git changes detected - objects serialise to identical YAML");
                            }
                            Err(e) => {
                                println!("Warning: Failed to check for changes: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        println!("Warning: Failed to initialise Git repository for instance: {e}");
                    }
                }
            }

            // Without --strict a partial pull still exits zero, so existing scripts
            // keep working. With it, a scheduled job can tell that some content types
            // were not retrieved.
            if strict && !failed_content_types.is_empty() {
                return Err(anyhow::anyhow!(
                    "{} content type(s) failed to pull: {}",
                    failed_content_types.len(),
                    failed_content_types.join(", ")
                ));
            }
        }
        ModuleCommands::Diff { instance } => {
            let explicit = instance.is_some();
            let instance_name = instance.unwrap_or_else(|| "default".to_string());
            require_instance(&instance_name, explicit)?;

            // Hold the instance lock: diff reads every local file, and a concurrent
            // pull rewrites them underneath it.
            let _lock = InstanceLock::acquire(&instance_name)?;

            let config_manager = ConfigManager::new();
            let module_config = config_manager.load_module_config(&instance_name, module_id)?;

            // Check if module is enabled
            if !module_config.enabled {
                println!("Module '{module_id}' is disabled in instance '{instance_name}'. Enable it in config.toml to use this command.");
                return Ok(());
            }

            let module_client = api::ModuleClient::new(module_config, module.base_api_path());
            let yaml_parser = YamlParser::new();

            // Get local files from the module-specific directory
            let module_dir = format!(
                "{}/{}",
                instance_name,
                module_dir_name(&instance_name, module)
            );

            // Get content type names from the module definition
            let content_type_names: Vec<&str> =
                module.content_types().iter().map(|ct| ct.name).collect();

            let local_files = yaml_parser.get_local_files(&module_dir, &content_type_names)?;

            if local_files.is_empty() {
                println!("No local YAML files found for module '{module_id}' in instance '{instance_name}'");
                println!("Run 'gcgit {module_id} pull --instance {instance_name}' to fetch configurations first");
                return Ok(());
            }

            // Group local files by content type so each content type is pulled from
            // the platform exactly once. Comparing per file previously re-pulled the
            // whole content type for every file, which for scripts meant one request
            // per script per file.
            let mut files_by_content_type: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for file_path in local_files {
                let local_content = yaml_parser.parse_file(&file_path)?;
                files_by_content_type
                    .entry(local_content.content_type.clone())
                    .or_default()
                    .push(file_path);
            }

            let content_types = module.content_types();
            let mut differences_found = false;
            let mut comparison_failures = 0usize;

            for (content_type, file_paths) in files_by_content_type {
                let Some(content_def) = content_types.iter().find(|ct| ct.name == content_type)
                else {
                    println!("[WARN] Content type '{content_type}' is not defined by module '{module_id}'; skipping {} file(s)", file_paths.len());
                    comparison_failures += 1;
                    continue;
                };

                let remote_objects = match module_client.pull_content_type_by_id(content_def).await
                {
                    Ok(objects) => objects,
                    Err(e) => {
                        // A failed pull says nothing about whether the local objects
                        // still exist remotely. Reporting them as new would be a guess.
                        println!("[ERROR] Could not compare {content_type} - {e}");
                        println!(
                            "  {} local file(s) for this content type were not checked",
                            file_paths.len()
                        );
                        comparison_failures += 1;
                        continue;
                    }
                };

                let mut matched_remote_ids: std::collections::HashSet<String> =
                    std::collections::HashSet::new();

                for file_path in file_paths {
                    let local_content = yaml_parser.parse_file(&file_path)?;

                    let Some(remote_content) = remote_objects.get(&local_content.id) else {
                        differences_found = true;
                        println!(
                            "LOCAL ONLY: {file_path} (no object with id '{}' on the platform)",
                            local_content.id
                        );
                        continue;
                    };
                    matched_remote_ids.insert(remote_content.id.clone());

                    match yaml_parser.objects_are_logically_equal(&local_content, remote_content) {
                        Ok(true) => {}
                        Ok(false) => {
                            differences_found = true;
                            println!("DIFF: {file_path} (local differs from remote)");
                            show_object_differences(&local_content, remote_content);
                        }
                        Err(e) => {
                            comparison_failures += 1;
                            println!("[ERROR] {file_path} (comparison failed: {e})");
                            if local_content != *remote_content {
                                differences_found = true;
                                println!("DIFF: {file_path} (local differs from remote - fallback comparison)");
                            }
                        }
                    }
                }

                // Objects on the platform with no local file. Previously invisible:
                // diff only ever walked local files.
                let mut remote_only: Vec<&String> = remote_objects
                    .values()
                    .filter(|object| !matched_remote_ids.contains(&object.id))
                    .map(|object| &object.id)
                    .collect();
                remote_only.sort();
                remote_only.dedup();

                for id in remote_only {
                    differences_found = true;
                    println!(
                        "REMOTE ONLY: {content_type}/{id} (on the platform but not stored locally)"
                    );
                }
            }

            if comparison_failures > 0 {
                println!(
                    "\n{comparison_failures} content type(s) or file(s) could not be compared"
                );
            }

            // Provide feedback when no differences are found
            if !differences_found && comparison_failures == 0 {
                println!(
                    "No differences detected - local YAML files match remote {} objects",
                    module_id.to_uppercase()
                );
            }
        }
        ModuleCommands::Test { instance } => {
            // Unlike pull and diff, test falls back to environment variables when an
            // instance has no configuration, so a missing instance is only an error
            // when one was named explicitly.
            if let Some(name) = &instance {
                require_instance(name, true)?;
            }
            let instance_name = instance.unwrap_or_else(|| "default".to_string());

            let config_manager = ConfigManager::new();
            let module_config = match config_manager.load_module_config(&instance_name, module_id) {
                Ok(config) => {
                    // Check if module is enabled
                    if !config.enabled {
                        println!("Module '{module_id}' is disabled in instance '{instance_name}'. Enable it in config.toml to use this command.");
                        return Ok(());
                    }
                    config
                }
                Err(_) => {
                    println!("Module '{module_id}' configuration not found for instance '{instance_name}'. Trying environment variables...");

                    // Fallback to environment variables if instance config doesn't exist
                    match ConfigManager::create_test_config() {
                        Ok(config) => {
                            // Convert XsiamConfig to ModuleConfig
                            let force_http =
                                crate::config::resolve_force_http(config.force_http, module_id);
                            crate::config::ModuleConfig {
                                enabled: true,
                                fqdn: config.fqdn,
                                api_key: config.api_key,
                                api_key_id: config.api_key_id,
                                force_http,
                            }
                        }
                        Err(e) => {
                            println!("[ERROR] Configuration error: {e}");
                            println!("\nTo fix this, either:");
                            println!(
                                "  1. Create an instance: gcgit init --instance {instance_name}"
                            );
                            println!("  2. Set environment variables: XSIAM_FQDN, XSIAM_API_KEY, XSIAM_API_KEY_ID");
                            return Ok(());
                        }
                    }
                }
            };

            let module_client = api::ModuleClient::new(module_config, module.base_api_path());

            println!("Testing {} API connectivity...\n", module_id.to_uppercase());

            // Test connectivity
            match module_client
                .test_connectivity(module.connectivity_endpoint())
                .await
            {
                Ok(_) => {
                    println!("API connectivity test successful");

                    // Test each content type endpoint
                    let content_types = module.content_types();
                    let mut successful_endpoints = 0;
                    let total_endpoints = content_types.len();

                    for content_def in content_types {
                        print!("Testing {:<25} ", format!("{}:", content_def.name));

                        match module_client.pull_content_type(&content_def).await {
                            Ok(outcome) => {
                                let note = if outcome.complete { "" } else { ", partial" };
                                println!("[OK] {} items{note}", outcome.objects.len());
                                successful_endpoints += 1;
                            }
                            Err(e) => {
                                println!("[FAIL] {e}");
                            }
                        }
                    }

                    println!("\n{successful_endpoints}/{total_endpoints} endpoints available");

                    if successful_endpoints == total_endpoints {
                        println!(
                            "All {} module endpoints are operational",
                            module_id.to_uppercase()
                        );
                    } else if successful_endpoints > 0 {
                        println!("[INFO] Some endpoints unavailable (this may be normal depending on your licence)");
                    } else {
                        println!("[ERROR] No endpoints available - check your configuration");
                    }
                }
                Err(e) => {
                    println!("\n[ERROR] API connectivity test failed: {e}");
                }
            }
        }
        ModuleCommands::Delete {
            instance: _,
            content_type: _,
            id: _,
        } => {
            let module_upper = module_id.to_uppercase();
            eprintln!("[ERROR] Feature not yet available");
            eprintln!();
            eprintln!(
                "Usage: gcgit {module_id} delete --instance <NAME> --content-type <TYPE> --id <ID>"
            );
            eprintln!();
            eprintln!("Delete operations for {module_upper} are still under development.");
            eprintln!("Visit https://gocortex.io for updates on feature availability.");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn handle_init_command(instance: String, force: bool) -> Result<()> {
    let config_manager = ConfigManager::new();
    config_manager.init_instance(&instance, force)?;

    println!("Initialised instance: {instance}");
    println!("Please edit {instance}/config.toml with your API credentials");
    println!("  [modules.platform]  Cortex Platform: dashboards, correlation rules, BIOCs,");
    println!("                      widgets, scripts, XQL library, RBAC, datasets, content packs");
    println!("  [modules.appsec]    Application Security");
    println!("  [modules.agent]     Agent Configurations");
    println!("  [modules.cwp]       Cloud Workload Protection");
    println!();
    println!("Then: gcgit platform pull --instance {instance}");

    Ok(())
}

async fn handle_status_command(instance: Option<String>) -> Result<()> {
    let config_manager = ConfigManager::new();

    match instance {
        Some(instance_name) => {
            println!("Status for instance: {instance_name}");
            show_instance_status(&config_manager, &instance_name).await?;
        }
        None => {
            println!("Status for all instances:");
            // Get all instance directories
            let instances = get_all_instances()?;
            for instance_name in instances {
                println!("\n=== {instance_name} ===");
                show_instance_status(&config_manager, &instance_name).await?;
            }
        }
    }

    Ok(())
}

async fn handle_validate_command(instance: Option<String>, files: Vec<String>) -> Result<()> {
    let yaml_parser = YamlParser::new();
    let module_registry = ModuleRegistry::load();

    // Collect all content type names from all modules for validation
    let all_content_types: Vec<&str> = module_registry
        .all_modules()
        .iter()
        .flat_map(|module| module.content_types())
        .map(|ct| ct.name)
        .collect();

    // Determine files to validate
    let files_to_validate = if !files.is_empty() {
        files
    } else if let Some(instance_name) = &instance {
        // Get all YAML files in the specified instance across all modules
        let mut instance_files = Vec::new();
        for module in module_registry.all_modules() {
            let module_dir = format!(
                "{}/{}",
                instance_name,
                module_dir_name(instance_name, module)
            );
            if let Ok(files) = yaml_parser.get_local_files(&module_dir, &all_content_types) {
                instance_files.extend(files);
            }
        }
        instance_files
    } else {
        // Get all YAML files in all instances
        let instances = get_all_instances()?;
        let mut all_files = Vec::new();
        for inst in instances {
            for module in module_registry.all_modules() {
                let module_dir = format!("{}/{}", inst, module_dir_name(&inst, module));
                if let Ok(files) = yaml_parser.get_local_files(&module_dir, &all_content_types) {
                    all_files.extend(files);
                }
            }
        }
        all_files
    };

    if files_to_validate.is_empty() {
        println!("No YAML files found to validate");
        return Ok(());
    }

    println!("Validating {} files...", files_to_validate.len());
    let mut validation_errors = 0;

    for file_path in files_to_validate {
        print!("  Checking {file_path}... ");

        match yaml_parser.parse_file(&file_path) {
            Ok(xsiam_object) => {
                // Validate content type is supported by checking against all registered modules
                if all_content_types.contains(&xsiam_object.content_type.as_str()) {
                    println!("Valid");
                } else {
                    println!(
                        "INVALID: Unsupported content type: {}",
                        xsiam_object.content_type
                    );
                    validation_errors += 1;
                }
            }
            Err(e) => {
                println!("[ERROR] {e}");
                validation_errors += 1;
            }
        }
    }

    if validation_errors > 0 {
        println!("\n{validation_errors} validation errors found");
        return Err(anyhow::anyhow!("Validation failed"));
    } else {
        println!("\nAll files are valid");
    }

    Ok(())
}

async fn show_instance_status(config_manager: &ConfigManager, instance_name: &str) -> Result<()> {
    // Check if instance exists
    if !std::path::Path::new(instance_name).exists() {
        println!("  Instance '{instance_name}' not found");
        return Ok(());
    }

    // Git status for this instance (using instance-specific git repo)
    match GitWrapper::new_for_instance(instance_name) {
        Ok(git_wrapper) => {
            let modified_files = git_wrapper.get_modified_files_in_current_repo()?;

            if modified_files.is_empty() {
                println!("  Git: No modified files");
            } else {
                println!("  Git: {} modified files", modified_files.len());
                for file in &modified_files {
                    println!("    - {file}");
                }
            }
        }
        Err(_) => {
            println!("  Git: No repository (run gcgit pull to initialise)");
        }
    }

    // Module connectivity status - check all enabled modules dynamically
    let module_registry = crate::modules::ModuleRegistry::load();
    for module in module_registry.all_modules() {
        let module_id = module.id();

        match config_manager.load_module_config(instance_name, module_id) {
            Ok(module_config) => {
                if module_config.enabled {
                    let module_client =
                        api::ModuleClient::new(module_config, module.base_api_path());
                    match module_client
                        .test_connectivity(module.connectivity_endpoint())
                        .await
                    {
                        Ok(_) => println!("  {}: Connected", module_id.to_uppercase()),
                        Err(e) => {
                            println!("  {}: Connection failed - {e}", module_id.to_uppercase())
                        }
                    }
                } else {
                    println!("  {}: Disabled", module_id.to_uppercase());
                }
            }
            Err(_) => {
                // Module not configured - skip silently
            }
        }
    }

    Ok(())
}

fn get_all_instances() -> Result<Vec<String>> {
    use std::fs;

    let mut instances = Vec::new();

    for entry in fs::read_dir(".")? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(dir_name) = path.file_name() {
                if let Some(dir_str) = dir_name.to_str() {
                    // Check if this looks like an instance directory (has config.toml)
                    let config_path = path.join("config.toml");
                    if config_path.exists() {
                        instances.push(dir_str.to_string());
                    }
                }
            }
        }
    }

    Ok(instances)
}

use crate::types::XsiamObject;

/// Display a detailed summary of differences between local and remote objects
fn show_object_differences(local: &XsiamObject, remote: &XsiamObject) {
    let mut differences = Vec::new();

    // Check basic field differences
    if local.id != remote.id {
        differences.push(format!("  -> ID: '{}' -> '{}'", local.id, remote.id));
    }
    if local.name != remote.name {
        let local_name = local.name.as_deref().unwrap_or(&local.id);
        let remote_name = remote.name.as_deref().unwrap_or(&remote.id);
        differences.push(format!(
            "  -> Name: '{}' -> '{}'",
            truncate_string(local_name, 30),
            truncate_string(remote_name, 30)
        ));
    }
    if local.description != remote.description {
        differences.push(format!(
            "  -> Description: {} chars -> {} chars",
            local.description.len(),
            remote.description.len()
        ));
    }
    if local.content_type != remote.content_type {
        differences.push(format!(
            "  -> Type: '{}' -> '{}'",
            local.content_type, remote.content_type
        ));
    }

    // Check content differences
    let content_diffs = analyse_content_differences(&local.content, &remote.content);
    differences.extend(content_diffs);

    // Display differences with helpful formatting
    if differences.is_empty() {
        println!("  -> No functional differences detected (metadata-only changes)");
    } else {
        for diff in &differences {
            println!("{diff}");
        }

        // Show helpful action suggestions
        if differences.len() > 1 {
            println!("  -> {} changes detected", differences.len());
        }

        // Equality is now byte equality of the written form, so reaching this
        // function already means the file will change. The previous branch that
        // reported "structural differences only" could not be reached.
        println!("  -> File content will change on next pull");
    }
}

/// Analyse differences in content HashMap
fn analyse_content_differences(
    local: &std::collections::BTreeMap<String, serde_json::Value>,
    remote: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<String> {
    let mut differences = Vec::new();

    // Find keys that exist in both
    let mut all_keys: std::collections::HashSet<String> = local.keys().cloned().collect();
    all_keys.extend(remote.keys().cloned());

    let mut modified_keys = Vec::new();
    let mut added_keys = Vec::new();
    let mut removed_keys = Vec::new();

    for key in all_keys {
        match (local.get(&key), remote.get(&key)) {
            (Some(local_val), Some(remote_val)) => {
                if local_val != remote_val {
                    modified_keys.push(key);
                }
            }
            (None, Some(_)) => added_keys.push(key),
            (Some(_), None) => removed_keys.push(key),
            (None, None) => {} // Shouldn't happen
        }
    }

    // Format the differences with helpful summaries
    if !added_keys.is_empty() {
        if added_keys.len() <= 3 {
            differences.push(format!("  -> Added fields: {}", added_keys.join(", ")));
        } else {
            differences.push(format!(
                "  -> Added {} new fields: {}, ...",
                added_keys.len(),
                added_keys[..2].join(", ")
            ));
        }
    }

    if !removed_keys.is_empty() {
        if removed_keys.len() <= 3 {
            differences.push(format!("  -> Removed fields: {}", removed_keys.join(", ")));
        } else {
            differences.push(format!(
                "  -> Removed {} fields: {}, ...",
                removed_keys.len(),
                removed_keys[..2].join(", ")
            ));
        }
    }

    if !modified_keys.is_empty() {
        if modified_keys.len() <= 3 {
            differences.push(format!(
                "  -> Modified fields: {}",
                modified_keys.join(", ")
            ));
        } else {
            differences.push(format!(
                "  -> Modified {} fields: {}, ...",
                modified_keys.len(),
                modified_keys[..2].join(", ")
            ));
        }
    }

    differences
}

/// Truncate string for display purposes.
///
/// Counts and slices by character rather than by byte. Slicing a `&str` by byte
/// offset panics when the offset falls inside a multi-byte character, so any
/// object named in a non-ASCII script would previously crash the diff command.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        s.chars().take(max_len).collect()
    } else {
        let truncated: String = s.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn truncate_string_handles_multibyte_characters() {
        // This exists because slicing by byte offset panics when the offset lands
        // inside a multi-byte character, which crashed diff for any non-ASCII object
        // name. The inputs are built so the cut point falls inside a character rather
        // than between two: an input that merely contains a multi-byte character
        // passes against the broken implementation and tests nothing.
        //
        // Characters are constructed rather than written literally, so the source
        // stays ASCII.
        let two_byte = char::from_u32(0x00dc).unwrap(); // capital U with diaeresis
        let three_byte = char::from_u32(0x4e2d).unwrap(); // CJK ideograph

        // truncate_string(s, 30) took &s[..27]. Twenty six ASCII characters put the
        // two-byte character across bytes 26 and 27, so byte 27 is inside it.
        let straddles_at_27 = format!(
            "{}{}{}",
            "abcdefghijklmnopqrstuvwxyz", two_byte, "trailing text to force truncation"
        );
        let truncated = truncate_string(&straddles_at_27, 30);
        assert!(truncated.chars().count() <= 30);
        assert!(truncated.ends_with("..."));

        // The same for a three-byte character, which spans bytes 25 to 27.
        let straddles_wider = format!(
            "{}{}{}",
            "abcdefghijklmnopqrstuvwxy", three_byte, "trailing text to force truncation"
        );
        let truncated = truncate_string(&straddles_wider, 30);
        assert!(truncated.chars().count() <= 30);

        // A short limit, where the cut lands inside the first character.
        let leading = format!("{}{}", three_byte, "abcdefghijklmnop");
        let truncated = truncate_string(&leading, 5);
        assert!(truncated.chars().count() <= 5);
    }

    #[test]
    fn truncate_string_leaves_short_input_untouched() {
        assert_eq!(truncate_string("short", 30), "short");
        let multibyte = char::from_u32(0x00dc).unwrap().to_string();
        assert_eq!(truncate_string(&multibyte, 30), multibyte);
    }

    #[test]
    fn truncate_string_handles_tiny_limits() {
        assert_eq!(truncate_string("abcdef", 2).chars().count(), 2);
        assert_eq!(truncate_string("abcdef", 0), "");
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn a_missing_instance_is_reported_before_anything_is_attempted() {
        let base = std::env::temp_dir().join(format!("gcgit_inst_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let missing = base.join("nope");
        let name = missing.to_str().unwrap();

        // Named explicitly: say so plainly.
        let err = require_instance(name, true).unwrap_err().to_string();
        assert!(err.contains("does not exist"), "unexpected: {err}");
        assert!(
            err.contains("gcgit init"),
            "should say how to fix it: {err}"
        );

        // Fell back to the default: say that no instance was given, because the user
        // never typed the name that appears in the message.
        let err = require_instance(name, false).unwrap_err().to_string();
        assert!(err.contains("No instance given"), "unexpected: {err}");
        assert!(
            err.contains("--instance"),
            "should point at the flag: {err}"
        );
    }

    #[test]
    fn a_directory_without_a_config_is_distinguished_from_a_missing_one() {
        let base = std::env::temp_dir().join(format!("gcgit_inst2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let name = base.to_str().unwrap();

        let err = require_instance(name, true).unwrap_err().to_string();
        assert!(err.contains("no config.toml"), "unexpected: {err}");

        std::fs::write(base.join("config.toml"), "instance_name = \"x\"\n").unwrap();
        assert!(
            require_instance(name, true).is_ok(),
            "a configured instance should pass"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_bare_name_applies_to_every_module() {
        let s = ContentTypeSelector::parse("policies");
        assert_eq!(s.module, None);
        assert!(s.matches("cwp", "policies"));
        assert!(s.matches("appsec", "policies"));
        assert!(!s.matches("cwp", "dashboards"));
    }

    #[test]
    fn a_qualified_name_applies_to_one_module_only() {
        // CWP and AppSec both have "policies"; qualifying picks one.
        let s = ContentTypeSelector::parse("cwp/policies");
        assert_eq!(s.module.as_deref(), Some("cwp"));
        assert!(s.matches("cwp", "policies"));
        assert!(
            !s.matches("appsec", "policies"),
            "must not reach the other module"
        );
        assert!(!s.matches("cwp", "registry_onboarding"));
    }

    #[test]
    fn a_qualified_name_tolerates_spacing() {
        let s = ContentTypeSelector::parse(" platform / attack_surface_rules ");
        assert_eq!(s.module.as_deref(), Some("platform"));
        assert_eq!(s.content_type, "attack_surface_rules");
        assert!(s.matches("platform", "attack_surface_rules"));
    }

    #[test]
    fn a_repository_path_is_a_valid_selector() {
        // The separator is the one the working tree already uses, so a path copied
        // from the repository can be pasted straight in.
        let s = ContentTypeSelector::parse("platform/attack_surface_rules");
        assert!(s.matches("platform", "attack_surface_rules"));
    }

    #[test]
    fn skip_accepts_values_separated_by_spaces() {
        use clap::Parser;
        // A list typed with a space after each comma is split by the shell into
        // separate arguments before clap sees it. Without num_args those arrive as
        // positionals and are rejected.
        let cli = Cli::try_parse_from(argv(&[
            "gcgit",
            "platform",
            "pull",
            "--skip",
            "platform/attack_surface_rules,",
            "platform/xql_library,",
            "platform/datasets",
        ]))
        .expect("values separated by spaces should parse");
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { skip, .. },
            }) => {
                // Each trailing comma splits into an empty value, so the raw count is
                // not the interesting property. What matters is what survives cleaning.
                let cleaned: Vec<String> = skip
                    .into_iter()
                    .map(|r| r.trim().trim_matches(',').trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect();
                assert_eq!(
                    cleaned,
                    vec![
                        "platform/attack_surface_rules".to_string(),
                        "platform/xql_library".to_string(),
                        "platform/datasets".to_string(),
                    ]
                );
            }
            _ => panic!("expected a platform pull"),
        }
    }

    #[test]
    fn a_following_flag_ends_the_skip_list() {
        use clap::Parser;
        // A greedy list must not swallow the flags after it.
        let cli = Cli::try_parse_from(argv(&[
            "gcgit",
            "platform",
            "pull",
            "--skip",
            "datasets",
            "--instance",
            "prod",
            "--quiet",
        ]))
        .expect("a following flag should end the list");
        match cli.command {
            Some(Commands::Platform {
                command:
                    ModuleCommands::Pull {
                        skip,
                        instance,
                        quiet,
                        ..
                    },
            }) => {
                assert_eq!(skip, vec!["datasets".to_string()]);
                assert_eq!(instance.as_deref(), Some("prod"));
                assert!(quiet);
            }
            _ => panic!("expected a platform pull"),
        }
    }

    #[test]
    fn skip_accepts_a_comma_list_and_repetition() {
        use clap::Parser;

        let cli = Cli::try_parse_from(argv(&[
            "gcgit",
            "platform",
            "pull",
            "--skip",
            "attack_surface_rules,datasets",
        ]))
        .expect("comma separated --skip should parse");
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { skip, .. },
            }) => {
                assert_eq!(
                    skip,
                    vec!["attack_surface_rules".to_string(), "datasets".to_string()]
                )
            }
            _ => panic!("expected a platform pull"),
        }

        let cli = Cli::try_parse_from(argv(&[
            "gcgit", "platform", "pull", "--skip", "datasets", "--skip", "widgets",
        ]))
        .expect("repeated --skip should parse");
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { skip, .. },
            }) => {
                assert_eq!(skip, vec!["datasets".to_string(), "widgets".to_string()])
            }
            _ => panic!("expected a platform pull"),
        }
    }

    #[test]
    fn content_type_also_accepts_a_comma_list() {
        use clap::Parser;
        let cli = Cli::try_parse_from(argv(&[
            "gcgit",
            "platform",
            "pull",
            "--content-type",
            "dashboards,widgets",
        ]))
        .expect("comma separated --content-type should parse");
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { content_type, .. },
            }) => {
                assert_eq!(
                    content_type,
                    vec!["dashboards".to_string(), "widgets".to_string()]
                )
            }
            _ => panic!("expected a platform pull"),
        }
    }

    #[test]
    fn skip_is_empty_by_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(argv(&["gcgit", "platform", "pull"])).unwrap();
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { skip, .. },
            }) => {
                assert!(skip.is_empty())
            }
            _ => panic!("expected a platform pull"),
        }
    }

    #[test]
    fn pull_flags_parse_in_long_and_short_form() {
        use clap::Parser;

        let cli = Cli::try_parse_from(argv(&[
            "gcgit",
            "platform",
            "pull",
            "--instance",
            "prod",
            "--quiet",
        ]))
        .expect("--quiet should parse");
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { quiet, .. },
            }) => {
                assert!(quiet)
            }
            _ => panic!("expected a platform pull"),
        }

        let cli = Cli::try_parse_from(argv(&["gcgit", "platform", "pull", "-q"]))
            .expect("-q should parse");
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { quiet, .. },
            }) => {
                assert!(quiet)
            }
            _ => panic!("expected a platform pull"),
        }

        // Absent by default, so existing invocations are unchanged.
        let cli = Cli::try_parse_from(argv(&["gcgit", "platform", "pull"])).unwrap();
        match cli.command {
            Some(Commands::Platform {
                command: ModuleCommands::Pull { quiet, .. },
            }) => {
                assert!(!quiet)
            }
            _ => panic!("expected a platform pull"),
        }
    }

    #[test]
    fn the_xsiam_alias_still_reaches_the_platform_module() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(argv(&["gcgit", "xsiam", "pull"])).expect("alias should parse");
        assert!(matches!(cli.command, Some(Commands::Platform { .. })));
    }

    #[test]
    fn a_single_module_takes_the_ordinary_path() {
        // Existing usage must be untouched by the list handling.
        assert_eq!(
            requested_module_list(&argv(&["gcgit", "platform", "pull"])),
            None
        );
        assert_eq!(requested_module_list(&argv(&["gcgit", "status"])), None);
        assert_eq!(requested_module_list(&argv(&["gcgit"])), None);
    }

    #[test]
    fn a_comma_list_is_split_in_order() {
        assert_eq!(
            requested_module_list(&argv(&["gcgit", "appsec,platform,agent", "pull"])),
            Some(vec![
                "appsec".to_string(),
                "platform".to_string(),
                "agent".to_string()
            ])
        );
    }

    #[test]
    fn a_list_tolerates_spacing_and_repeats() {
        assert_eq!(
            requested_module_list(&argv(&["gcgit", "platform, appsec ,platform", "pull"])),
            Some(vec!["platform".to_string(), "appsec".to_string()])
        );
    }

    #[test]
    fn a_flag_in_the_command_position_is_not_a_module_list() {
        // --help and --version carry no comma, but a future flag might.
        assert_eq!(
            requested_module_list(&argv(&["gcgit", "--some,flag"])),
            None
        );
    }

    #[test]
    fn an_empty_list_is_not_treated_as_a_request() {
        assert_eq!(
            requested_module_list(&argv(&["gcgit", ",,,", "pull"])),
            None
        );
    }

    #[test]
    fn an_existing_legacy_directory_keeps_being_used() {
        // An instance created before the rename must keep writing to its original
        // directory, or its backup appears to vanish and starts again empty.
        let registry = ModuleRegistry::load();
        let platform = registry.get("platform").unwrap();

        let base = std::env::temp_dir().join(format!("gcgit_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("xsiam")).unwrap();
        let instance = base.to_str().unwrap();

        assert_eq!(module_dir_name(instance, platform), "xsiam");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_new_instance_uses_the_current_directory_name() {
        let registry = ModuleRegistry::load();
        let platform = registry.get("platform").unwrap();

        let base = std::env::temp_dir().join(format!("gcgit_newdir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let instance = base.to_str().unwrap();

        assert_eq!(module_dir_name(instance, platform), "platform");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_module_that_was_never_renamed_is_unaffected() {
        let registry = ModuleRegistry::load();
        let appsec = registry.get("appsec").unwrap();
        assert_eq!(appsec.legacy_id(), None);
        assert_eq!(module_dir_name("/nonexistent", appsec), "appsec");
    }

    #[test]
    fn sanitize_name_strips_control_characters() {
        // A live tenant returned an XQL library entry whose name ended in two
        // newlines, producing a file whose name contained literal newlines.
        let name = "Latest Unresolved Critical and High Severity Cases by Score\n\n";
        let cleaned = sanitize_name_for_filename(name).expect("name should survive cleaning");
        assert!(
            !cleaned.chars().any(|c| c.is_control()),
            "control characters remain: {cleaned:?}"
        );
        assert!(
            !cleaned.ends_with('_'),
            "trailing separators should be trimmed: {cleaned:?}"
        );
        assert_eq!(
            cleaned,
            "Latest_Unresolved_Critical_and_High_Severity_Cases_by_Score"
        );
    }

    #[test]
    fn sanitize_name_replaces_path_and_shell_hostile_characters() {
        assert_eq!(
            sanitize_name_for_filename("a/b\\c:d*e?f"),
            Some("a_b_c_d_e_f".to_string())
        );
        assert_eq!(
            sanitize_name_for_filename("rule <prod>"),
            Some("rule__prod".to_string())
        );
    }

    #[test]
    fn sanitize_name_rejects_names_with_nothing_usable() {
        // The caller falls back to the object ID when None comes back.
        assert_eq!(sanitize_name_for_filename("   "), None);
        assert_eq!(sanitize_name_for_filename("\n\t"), None);
        assert_eq!(sanitize_name_for_filename("..."), None);
        assert_eq!(sanitize_name_for_filename(""), None);
    }

    #[test]
    fn sanitize_name_keeps_ordinary_names_intact() {
        assert_eq!(
            sanitize_name_for_filename("NGFW App-ID Stacking"),
            Some("NGFW_App-ID_Stacking".to_string())
        );
    }

    #[test]
    fn names_differing_only_in_case_are_treated_as_colliding() {
        // Reproduces a live pair: two XQL library entries whose names differ only in
        // the case of one letter. On a case-insensitive filesystem they would
        // resolve to the same file, so both must be disambiguated by ID.
        let names = [
            "Oracle_credentials_detected_in_code",
            "Oracle_Credentials_detected_in_code",
        ];
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for n in &names {
            *counts.entry(n.to_lowercase()).or_insert(0) += 1;
        }
        assert_eq!(counts.len(), 1, "the two names must map to one bucket");
        assert_eq!(
            counts.values().next(),
            Some(&2),
            "both must be seen as colliding"
        );
    }

    #[test]
    fn sanitize_id_strips_path_separators() {
        assert_eq!(
            sanitize_id_for_filename("../../etc/passwd"),
            "______etc_passwd"
        );
        assert_eq!(sanitize_id_for_filename("safe-id_1"), "safe-id_1");
    }

    #[test]
    fn prune_removes_only_files_absent_from_the_pull() {
        let dir = std::env::temp_dir().join(format!("gcgit_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("kept.yaml"), "id: kept").unwrap();
        std::fs::write(dir.join("stale.yaml"), "id: stale").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a config file").unwrap();

        let mut keep = HashSet::new();
        keep.insert("kept.yaml".to_string());

        let removed = prune_stale_files(dir.to_str().unwrap(), &keep).unwrap();

        assert_eq!(removed, vec!["stale.yaml".to_string()]);
        assert!(dir.join("kept.yaml").exists());
        assert!(!dir.join("stale.yaml").exists());
        // Non-YAML files are left alone.
        assert!(dir.join("notes.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_on_a_missing_directory_is_not_an_error() {
        let removed = prune_stale_files("definitely/not/a/real/path", &HashSet::new()).unwrap();
        assert!(removed.is_empty());
    }
}
