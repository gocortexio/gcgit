# gcgit - Git for Cortex XSIAM

Go Cortex Git is a Rust-based command-line interface (CLI) tool designed to serve as a lightweight abstraction layer between local Git operations and the Cortex XSIAM REST API. Its purpose is to enable security teams to version-control and deploy Cortex XSIAM configuration objects—such as Correlation Searches, Dashboards, BIOCs, and Scripts—without requiring a full-scale CI/CD pipeline or remote Git hosting. By wrapping standard Git workflows and translating file changes into corresponding API actions, gcgit streamlines content management within XSIAM while keeping everything local and traceable via Git.

## Quick Start

### 1. Install gcgit
Download the latest release or build from source:
```bash
cargo build --release
```

### 2. Create an Instance
```bash
gcgit init --instance myinstance
```

This creates a directory structure:
```
myinstance/
├── .git/                    # Git repository for this instance
├── config.toml              # XSIAM API credentials
├── biocs/                   # BIOC configurations
├── correlation_searches/    # Correlation rule configurations
├── dashboards/             # Dashboard configurations
└── widgets/                # Widget configurations
```

### 3. Configure API Access
Edit `myinstance/config.toml`:
```toml
[xsiam]
fqdn = "api-myinstance.xdr.au.paloaltonetworks.com"
api_key = "your-api-key"
api_key_id = "your-api-key-id"
instance_name = "myinstance"
```

### 4. Pull Configurations
```bash
gcgit xsiam pull --instance myinstance
```

This downloads all configurations from XSIAM and automatically commits them to the local Git repository.

## Commands

### Instance Management
```bash
# Create a new instance
gcgit init --instance myinstance

# Check status of local vs remote changes
gcgit status --instance myinstance
```

### XSIAM Operations
```bash
# Pull all configurations from XSIAM (auto-commits to Git)
gcgit xsiam pull --instance myinstance

# Show differences between local and remote
gcgit xsiam diff --instance myinstance

# Push local changes to XSIAM
gcgit xsiam push --instance myinstance
```

### Validation & Deployment
```bash
# Validate YAML files
gcgit validate --instance myinstance

# Complete deployment workflow: validate → add → commit → push
gcgit deploy --instance myinstance
```



## Configuration Types

gcgit supports five types of XSIAM objects:

- **correlation_searches**: Correlation rules and searches
- **dashboards**: Custom dashboards and visualisations
- **biocs**: Behavioural Indicators of Compromise
- **widgets**: Interactive widgets and visual components
- **authentication_settings**: SSO integration and authentication configurations

Each type is stored in its own subdirectory with YAML files.

### Testing API Connectivity
```bash
# Test connection to XSIAM API
gcgit xsiam test --instance myinstance
```

## Example Workflow

```bash
# Set up instance
gcgit init --instance myinstance
# Edit myinstance/config.toml with your credentials

# Pull existing configurations
gcgit xsiam pull --instance myinstance

# Make local changes to YAML files
# Edit myinstance/correlation_searches/my-rule.yaml

# Validate and deploy changes
gcgit validate --instance myinstance
gcgit deploy --instance myinstance
```

## Key Features

- **Instance Isolation**: Each XSIAM instance has its own Git repository
- **Automatic Commits**: Pull operations automatically commit changes for version history
- **YAML-based**: All configurations stored as readable YAML files
- **Version Control**: Full Git integration with change tracking
- **Validation**: Pre-deployment YAML validation
- **Multi-instance**: Manage multiple XSIAM environments

## Requirements

- Rust 1.70+ (for building from source)
- Git
- Cortex XSIAM API access (API key and key ID)

## Building from Source

```bash
git clone <repository-url>
cd gcgit
cargo build --release
./target/release/gcgit --version
```

## Help

```bash
gcgit --help                    # Main help
gcgit xsiam --help             # XSIAM command help
gcgit init --help              # Instance creation help
```