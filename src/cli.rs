// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gcgit")]
#[command(
    about = "A Rust-based CLI tool for version-controlling Cortex platform configurations (XSIAM, AppSec, Agent, CWP).\nSynchronise YAML-based configuration files between local Git repositories and Cortex instances.\n\nhttps://gocortex.io"
)]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(long_about = concat!("A Rust-based CLI tool for version-controlling Cortex platform configurations.\nSupports multiple Cortex modules: XSIAM, Application Security, Agent Configurations, Cloud Workload Protection.\n\nhttps://gocortex.io\n\nVersion: ", env!("CARGO_PKG_VERSION")))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Cortex Platform module commands (dashboards, correlation rules, BIOCs, widgets,
    /// scripts, XQL library, RBAC, datasets, content packs)
    ///
    /// Accepts "xsiam" as an alias. That was the command name before the module was
    /// renamed, and existing scripts and pipelines continue to work.
    #[command(alias = "xsiam")]
    Platform {
        #[command(subcommand)]
        command: ModuleCommands,
    },
    /// AppSec module commands (applications, policies, rules, repositories, integrations)
    Appsec {
        #[command(subcommand)]
        command: ModuleCommands,
    },
    /// Agent Configurations module commands (10 global agent settings singletons)
    Agent {
        #[command(subcommand)]
        command: ModuleCommands,
    },
    /// CWP module commands (Cloud Workload Protection: policies, registry onboarding)
    Cwp {
        #[command(subcommand)]
        command: ModuleCommands,
    },
    /// Initialise a new multi-module instance
    Init {
        /// Instance name
        #[arg(long)]
        instance: String,
        /// Replace an existing config.toml instead of refusing to overwrite it
        #[arg(long)]
        force: bool,
    },
    /// Show Git and module synchronisation status
    Status {
        /// Instance name to check (optional - shows all if not specified)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Streamlined deployment: validate + add + commit + push to platform
    ///
    /// Hidden: not implemented. The command exits with an error. Advertising a write
    /// path to the platform that does not exist is worse than omitting it, because a
    /// reader would reasonably believe gcgit can modify a production tenant.
    #[command(hide = true)]
    Deploy {
        /// Instance name to deploy
        #[arg(long)]
        instance: String,
        /// Commit message
        #[arg(short, long)]
        message: String,
        /// Files to add and commit (if not specified, adds all modified YAML files in instance)
        files: Vec<String>,
    },
    /// Validate YAML files for platform compatibility
    Validate {
        /// Instance name to validate
        #[arg(long)]
        instance: Option<String>,
        /// Specific files to validate (if not specified, validates all YAML files in instance)
        files: Vec<String>,
    },
}

// Generic module commands that work across all modules
#[derive(Subcommand)]
pub enum ModuleCommands {
    /// Push local changes to the platform
    ///
    /// Hidden: not implemented. Invoking it exits with an error.
    #[command(hide = true)]
    Push {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
    },
    /// Pull configurations from the platform
    Pull {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
        /// Exit with a non-zero status if any content type fails to pull
        #[arg(long)]
        strict: bool,
        /// Pull only the named content types. Repeatable, and accepts a comma
        /// separated list. Defaults to all.
        ///
        /// num_args allows values separated by spaces as well as commas, so a list
        /// typed with a space after each comma is not split by the shell into
        /// arguments clap then rejects. Pull takes no positional arguments, so there
        /// is nothing for a greedy list to swallow.
        #[arg(long = "content-type", value_delimiter = ',', num_args = 1..)]
        content_type: Vec<String>,
        /// Content types to leave out. Repeatable, and accepts a comma separated list.
        ///
        /// A bare name is skipped wherever it appears. Qualify it with a module to skip
        /// it in one place only, written the same way the repository stores it:
        /// --skip cwp/policies,platform/attack_surface_rules
        #[arg(long, value_delimiter = ',', num_args = 1..)]
        skip: Vec<String>,
        /// Report what would change without writing files or committing
        #[arg(long)]
        dry_run: bool,
        /// Write files but do not stage or commit them. Use when the surrounding
        /// workflow owns the commit, such as a scheduled job in a CI pipeline.
        #[arg(long)]
        no_git: bool,
        /// Report per content type rather than per file. Counts, warnings and errors
        /// are still shown; the line for each individual file is not.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Show differences between local and remote
    Diff {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
    },
    /// Test API connectivity
    Test {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
    },
    /// Delete an object from the platform
    ///
    /// Hidden: not implemented. Invoking it exits with an error.
    #[command(hide = true)]
    Delete {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
        /// Content type
        #[arg(long)]
        content_type: String,
        /// Object ID to delete
        #[arg(long)]
        id: String,
    },
}
