<div align="center">
  <img src="assets/gcgit-logo.png" alt="gcgit logo" width="600"/>
  
  # gcgit
  
  ![Version](https://img.shields.io/badge/version-2.1.8-blue)
  ![Rust](https://img.shields.io/badge/rust-1.70+-orange)
  
  **Version control for Cortex platform security configurations**
  
  A Git-based workflow for managing XSIAM and Application Security configurations using YAML files and local repositories.
</div>

---

## Table of Contents

- [Features](#features)
- [Supported Modules](#supported-modules)
- [Quick Start](#quick-start)
- [Commands](#commands)
- [Configuration](#configuration)
- [File Organisation](#file-organisation)
- [Git Integration](#git-integration)
- [Architecture](#architecture)
- [Security](#security)
- [Building](#building)

---

## Features

- **Multi-Module Support**: Manage XSIAM and Application Security from a single tool
- **Git-Based Workflow**: Automatic commits, change tracking, and version history
- **YAML Configuration**: Human-readable configuration files for easy editing and review
- **Module Architecture**: Plugin-based design enabling support for additional Cortex modules
- **Environment Variables**: Secure credential management with variable expansion
- **File Locking**: Prevents concurrent operations that could corrupt data
- **Zero Dependencies**: Self-contained binary using libgit2 (no external Git required)

### What's New in v2.x

Version 2.x represents a major architectural upgrade from the v1.x single-module design:

| v1.x | v2.x |
|------|------|
| XSIAM only | Multi-module (XSIAM + AppSec) |
| Fixed content types | Extensible plugin architecture |
| Monolithic design | Shared infrastructure with module traits |

---

## Supported Modules

### XSIAM Module

Six content types for security operations:

| Content Type | Description |
|--------------|-------------|
| `correlation_searches` | Detection and correlation rules |
| `dashboards` | Security dashboards and visualisations |
| `biocs` | Behavioural indicators of compromise |
| `widgets` | Dashboard components |
| `authentication_settings` | SSO and authentication configurations |
| `scripts` | Automation scripts |

### Application Security Module

Five content types for application security:

| Content Type | Description |
|--------------|-------------|
| `applications` | Application inventory and configuration |
| `policies` | Security policies and rules |
| `rules` | Detection rules |
| `repositories` | Code repository connections |
| `integrations` | Third-party integrations |

---

## Quick Start

### Installation

**Build from source:**

```bash
cargo build --release
./target/release/gcgit --version
```

### Basic Workflow

**1. Create an instance**

```bash
gcgit init --instance production
```

This creates the following structure:

```
production/
├── .git/                      # Local Git repository
├── config.toml                # API credentials
├── xsiam/                     # XSIAM module
│   ├── dashboards/
│   ├── correlation_searches/
│   └── ...
└── appsec/                    # AppSec module
    ├── applications/
    ├── policies/
    └── ...
```

**2. Configure API access**

Edit `production/config.toml`:

```toml
[modules.xsiam]
enabled = true
fqdn = "api-production.xdr.eu.paloaltonetworks.com"
api_key = "${XSIAM_API_KEY}"
api_key_id = "${XSIAM_API_KEY_ID}"

[modules.appsec]
enabled = true
fqdn = "api-production.xdr.eu.paloaltonetworks.com"
api_key = "${APPSEC_API_KEY}"
api_key_id = "${APPSEC_API_KEY_ID}"
```

Environment variables are expanded automatically using `${VARIABLE}` syntax.

**3. Pull configurations**

```bash
# Pull from XSIAM
gcgit xsiam pull --instance production

# Pull from AppSec
gcgit appsec pull --instance production
```

All changes are automatically committed to the Git repository with descriptive commit messages.

---

## Commands

### Instance Management

```bash
# Create new instance directory
gcgit init --instance <name>

# Check instance status
gcgit status --instance <name>

# Validate YAML files
gcgit validate --instance <name>
```

### Module Operations

Each module supports the same operations through a consistent interface:

```bash
# Pull configurations from remote platform
gcgit <module> pull --instance <name>

# Show differences between local and remote
gcgit <module> diff --instance <name>

# Test API connectivity
gcgit <module> test --instance <name>
```

Replace `<module>` with `xsiam` or `appsec`.

**Examples:**

```bash
# XSIAM operations
gcgit xsiam pull --instance production
gcgit xsiam diff --instance production
gcgit xsiam test --instance production

# AppSec operations
gcgit appsec pull --instance production
gcgit appsec diff --instance production
gcgit appsec test --instance production
```

### Development Status

| Status | Operations |
|--------|------------|
| **Production-Ready** | Pull, Diff, Test, Validate |
| **Under Development** | Push, Delete, Deploy |

Push functionality is being thoroughly tested before release to prevent accidental configuration changes in production environments.

---

## Configuration

### Multi-Module Configuration

Each instance supports multiple modules through `[modules.module_name]` blocks:

```toml
[modules.xsiam]
enabled = true
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "your-xsiam-api-key"
api_key_id = "your-xsiam-key-id"

[modules.appsec]
enabled = false
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "your-appsec-api-key"
api_key_id = "your-appsec-key-id"
```

Set `enabled = false` to disable a module whilst keeping its configuration.

### Environment Variables

API keys can be stored in environment variables for security:

```bash
export XSIAM_API_KEY="your-key"
export XSIAM_API_KEY_ID="your-key-id"
```

Reference them in configuration:

```toml
api_key = "${XSIAM_API_KEY}"
api_key_id = "${XSIAM_API_KEY_ID}"
```

---

## File Organisation

Configurations are stored as YAML files in a structured hierarchy:

```
instance-name/
├── module-name/
│   └── content-type/
│       ├── object-id-1.yaml
│       ├── object-id-2.yaml
│       └── ...
└── config.toml
```

**Example:**

```
production/
├── xsiam/
│   ├── dashboards/
│   │   ├── security-overview.yaml
│   │   └── threat-analysis.yaml
│   └── correlation_searches/
│       └── suspicious-login.yaml
└── appsec/
    ├── applications/
    │   └── webapp-frontend.yaml
    └── policies/
        └── data-protection.yaml
```

Each YAML file contains the complete configuration for one object, making changes easy to track through Git.

---

## Git Integration

Every pull operation automatically creates a Git commit with details about what changed. This provides:

- **Change History**: Complete audit trail of all configuration changes
- **Diff Viewing**: See exactly what changed between versions
- **Rollback**: Revert to previous configurations using Git
- **Branching**: Create feature branches for testing configuration changes

The Git repository lives inside each instance directory, keeping everything self-contained.

---

## Architecture

### v1.x vs v2.x

**v1.x architecture:**
- Single module (XSIAM only)
- Fixed content type list
- Monolithic design

**v2.x architecture:**
- Module trait system with plugin support
- Shared infrastructure (Git, API client, YAML parser)
- Three reusable pull strategies (JsonCollection, Paginated, ZipArtifact)
- Self-contained modules with declarative content type definitions
- Easy extension for future Cortex platform modules

### Module System

Each module implements a simple interface:
- Content type definitions (endpoint, response format, ID field)
- Pull strategy selection (how to retrieve data)
- API base path

The tool handles the rest: API communication, Git operations, YAML serialisation, file locking, and change detection.

---

## Security

- **Local Storage**: API keys are stored locally in instance configuration files
- **Environment Variables**: Variable expansion prevents committing secrets to Git
- **HTTPS Only**: All API communication uses HTTPS with certificate validation
- **File Locking**: Prevents concurrent operations that could corrupt data

**Never commit API keys or credentials to version control repositories.**

---

## Building

### Requirements

- Rust 1.70 or later
- Cortex XSIAM and/or AppSec API access
- API key and key ID for each module

No external Git installation required - gcgit uses libgit2 for Git operations.

### Build from Source

```bash
git clone <repository-url>
cd gcgit
cargo build --release
./target/release/gcgit --version
```

The compiled binary is self-contained with no runtime dependencies.

### Help and Documentation

```bash
# Main help
gcgit --help

# Module-specific help
gcgit xsiam --help
gcgit appsec --help

# Command-specific help
gcgit init --help
gcgit validate --help
```

---

<div align="center">
  <sub>Built with Rust | Version 2.1.8</sub>
</div>
