<!-- SPDX-FileCopyrightText: GoCortexIO -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->

<p align="center">
  <img src="assets/gcgit-logo.png" alt="gcgit" width="600">
</p>

# gcgit

Git-based version control for Cortex platform security configurations.

## Overview

gcgit is a command-line tool that synchronises security configurations between Palo Alto Networks Cortex platform instances and local Git repositories. It pulls configurations from Cortex XSIAM and Cortex Cloud APIs, stores them as YAML files, and tracks all changes through Git.

What it does:

- Pulls security configurations from Cortex platform instances via REST APIs
- Stores each configuration object as an individual YAML file
- Commits all changes to a local Git repository with descriptive messages
- Supports multiple Cortex platform modules from a single tool
- Compares local configurations against remote platform state

Why it exists:

Cortex platform instances have no built-in version control for security configurations. gcgit fills this gap by providing a Git-based audit trail and change tracking mechanism that works across multiple Cortex modules and platform instances.

## Features

- Multi-module support for Cortex XSIAM and Cortex Cloud
- Automatic Git commits with change tracking and audit trail
- YAML-based configuration files for human-readable diffs
- Plugin architecture for adding new Cortex modules
- Environment variable expansion for secure credential management
- File locking to prevent concurrent operations
- Self-contained binary using libgit2 (no external Git installation required)
- Five reusable pull strategies: JsonCollection, Paginated, OffsetPaginated, ScriptCode, ZipArtifact

## Supported Modules

### Cortex XSIAM (9 content types)

| Content Type | Description |
|--------------|-------------|
| dashboards | Security dashboards and visualisations |
| biocs | Behavioural indicators of compromise |
| correlation_searches | Detection and correlation rules |
| widgets | Dashboard components |
| authentication_settings | SSO and authentication configurations |
| scripts | Automation scripts (two-step code retrieval) |
| scheduled_queries | XQL scheduled queries |
| xql_library | Reusable XQL query library |
| rbac_users | Role-based access control users |

### Cortex Cloud (7 content types)

| Content Type | Description |
|--------------|-------------|
| applications | Application inventory and configuration |
| policies | Security policies for threat detection |
| rules | Custom security rules |
| repositories | Code repository configurations |
| integrations | Third-party integrations |
| application_configuration | Singleton application configuration |
| application_criteria | Application filtering criteria |

## Quick Start

Build from source:

```bash
cargo build --release
./target/release/gcgit --version
```

Create an instance:

```bash
gcgit init --instance production
```

This creates the following structure:

```
production/
+-- .git/
+-- config.toml
+-- xsiam/
|   +-- dashboards/
|   +-- correlation_searches/
|   +-- biocs/
|   +-- widgets/
|   +-- authentication_settings/
|   +-- scripts/
|   +-- scheduled_queries/
|   +-- xql_library/
|   +-- rbac_users/
+-- appsec/
    +-- applications/
    +-- policies/
    +-- rules/
    +-- repositories/
    +-- integrations/
    +-- application_configuration/
    +-- application_criteria/
```

Configure API access in production/config.toml:

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

Environment variables are expanded automatically using ${VARIABLE} syntax. gcgit also
recognises DEMISTO_BASE_URL, DEMISTO_API_KEY, and XSIAM_AUTH_ID as fallback variables
for cross-project compatibility.

Pull configurations:

```bash
gcgit xsiam pull --instance production
gcgit appsec pull --instance production
```

All changes are automatically committed to the local Git repository.

## Commands

| Command | Description |
|---------|-------------|
| init --instance NAME | Create a new instance directory with module subdirectories |
| xsiam pull --instance NAME | Pull all XSIAM configurations from the platform |
| xsiam diff --instance NAME | Show differences between local and remote |
| xsiam test --instance NAME | Test API connectivity to the XSIAM module |
| appsec pull --instance NAME | Pull all AppSec configurations from the platform |
| appsec diff --instance NAME | Show differences between local and remote |
| appsec test --instance NAME | Test API connectivity to the AppSec module |

Replace xsiam or appsec with any registered module name. Each module supports the same set of operations through a consistent interface.

### Development Status

| Status | Operations |
|--------|------------|
| Production-ready | pull, diff, test |
| Under development | push, delete, deploy |

Push operations are disabled pending thorough testing to prevent accidental modification of production configurations.

## Configuration

Each instance has a config.toml file with per-module credential blocks:

```toml
[modules.xsiam]
enabled = true
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "${XSIAM_API_KEY}"
api_key_id = "${XSIAM_API_KEY_ID}"

[modules.appsec]
enabled = false
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "${APPSEC_API_KEY}"
api_key_id = "${APPSEC_API_KEY_ID}"
```

Set enabled = false to disable a module whilst keeping its configuration. Each module can use different API credentials and even different platform FQDNs.

Store API keys in environment variables rather than directly in config.toml to prevent credentials from being committed to Git.

### Fallback Variables

If the primary environment variables are empty or unset, gcgit checks these fallback variables for cross-project compatibility:

| Primary Field | Fallback Variable | Notes |
|---------------|-------------------|-------|
| fqdn | DEMISTO_BASE_URL | https:// prefix and trailing slash are stripped automatically |
| api_key | DEMISTO_API_KEY | Used as-is |
| api_key_id | XSIAM_AUTH_ID | Used as-is |

When a fallback is used, gcgit prints an informational message to the console.

## File Organisation

Configurations are stored as individual YAML files in a structured hierarchy:

```
instance-name/
+-- config.toml
+-- module-name/
    +-- content-type/
        +-- object-id.yaml
```

Each YAML file contains the complete configuration for one object. Changes to individual objects produce clean, readable Git diffs.

## Building

Requirements:

- Rust 1.70 or later
- Cortex XSIAM and/or Cortex Cloud API access
- API key and key ID for each module

No external Git installation is required. gcgit uses libgit2 for all Git operations.

```bash
git clone <repository-url>
cd gcgit
cargo build --release
./target/release/gcgit --version
```

The compiled binary is self-contained with no runtime dependencies.

```bash
gcgit --help
gcgit xsiam --help
gcgit appsec --help
```

## Licence

This project is licensed under the GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later). See the [LICENSE](LICENSE) file for the full licence text.