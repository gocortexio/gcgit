// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gcgit")]
#[command(about = "A Rust-based CLI tool for version-controlling Cortex platform configurations (XSIAM, AppSec).\nSynchronise YAML-based configuration files between local Git repositories and Cortex instances.\n\nhttps://gocortex.io")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(long_about = concat!("A Rust-based CLI tool for version-controlling Cortex platform configurations.\nSupports multiple Cortex modules: XSIAM, Application Security.\n\nhttps://gocortex.io\n\nVersion: ", env!("CARGO_PKG_VERSION")))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// XSIAM module commands (scripts, dashboards, biocs, correlation searches, widgets, authentication settings)
    Xsiam {
        #[command(subcommand)]
        command: ModuleCommands,
    },
    /// AppSec module commands (applications, policies, rules, repositories, integrations)
    Appsec {
        #[command(subcommand)]
        command: ModuleCommands,
    },
    /// Initialise a new multi-module instance
    Init {
        /// Instance name
        #[arg(long)]
        instance: String,
    },
    /// Show Git and module synchronisation status
    Status {
        /// Instance name to check (optional - shows all if not specified)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Streamlined deployment: validate + add + commit + push to platform
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
