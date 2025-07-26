use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gcgit")]
#[command(about = "A Rust-based CLI tool for version-controlling Cortex XSIAM configurations.\nSynchronise YAML-based configuration files between local Git repositories and Cortex XSIAM instances.\n\nhttps://gocortex.io")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(long_about = concat!("A Rust-based CLI tool for version-controlling Cortex XSIAM configurations.\nSynchronise YAML-based configuration files between local Git repositories and Cortex XSIAM instances.\n\nhttps://gocortex.io\n\nVersion: ", env!("CARGO_PKG_VERSION")))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// XSIAM-related commands
    Xsiam {
        #[command(subcommand)]
        command: XsiamCommands,
    },
    /// Initialize a new instance
    Init {
        /// Instance name
        #[arg(long)]
        instance: String,
    },
    /// Show Git and XSIAM synchronization status
    Status {
        /// Instance name to check (optional - shows all if not specified)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Streamlined deployment: validate + add + commit + push to XSIAM
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
    /// Validate YAML files for XSIAM compatibility
    Validate {
        /// Instance name to validate
        #[arg(long)]
        instance: Option<String>,
        /// Specific files to validate (if not specified, validates all YAML files in instance)
        files: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum XsiamCommands {
    /// Push local changes to XSIAM
    Push {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
    },
    /// Pull configurations from XSIAM
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
    /// Delete an object from XSIAM
    Delete {
        /// Instance name
        #[arg(long)]
        instance: Option<String>,
        /// Content type (biocs, correlation_searches, dashboards)
        #[arg(long)]
        content_type: String,
        /// Object ID to delete
        #[arg(long)]
        id: String,
    },
}
