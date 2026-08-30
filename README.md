<!-- SPDX-FileCopyrightText: GoCortexIO -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->

<p align="center">
  <img src="assets/gcgit-logo.png" alt="gcgit" width="600">
</p>

# gcgit

Git-based version control for Cortex platform security configurations.

## Overview

gcgit is a command-line tool that synchronises security configurations between Palo Alto Networks Cortex platform instances and local Git repositories. It pulls configurations from Cortex XSIAM, Cortex Cloud AppSec, Agent Configurations, and Cortex Cloud Workload Protection (CWP) APIs, stores them as YAML files, and tracks all changes through Git.

What it does:

- Pulls security configurations from Cortex platform instances via REST APIs
- Stores each configuration object as an individual YAML file
- Commits all changes to a local Git repository with descriptive messages
- Supports multiple Cortex platform modules from a single tool
- Compares local configurations against remote platform state

Why it exists:

Cortex platform instances have no built-in version control for security configurations. gcgit fills this gap by providing a Git-based audit trail and change tracking mechanism that works across multiple Cortex modules and platform instances.

## Features

- Multi-module support for Cortex Platform, Cortex Cloud AppSec, Agent Configurations, and Cortex Cloud Workload Protection (CWP)
- 32 content types across four modules
- Automatic Git commits with change tracking and audit trail
- Deterministic YAML serialisation with idempotent re-pulls (server-bumped timestamp fields are excluded so unchanged objects produce no diffs)
- Local files are removed when the corresponding object is deleted on the platform, so the repository reflects current platform state
- Array ordering is preserved as the platform returns it, except for fields declared as
  sets on a content type because the platform returns their members in an arbitrary order
- Plugin architecture for adding new Cortex modules
- Environment variable expansion for secure credential management
- File locking to prevent concurrent operations
- Self-contained binary using libgit2 (no external Git installation required)
- Six reusable pull strategies: JsonCollection, Paginated, OffsetPaginated, BodyWindowPaginated, NameListed, ScriptCode
- All requests carry a gcgit User-Agent so the traffic is identifiable on the network and in platform request logs
- Graceful handling of licence-gated endpoints (a 403 on an optional endpoint is reported as a warning and the rest of the pull continues)
- Automatic retry with exponential backoff on rate limiting (HTTP 429) and server errors, honouring Retry-After
- Platform error messages are reported in full, including the specific permission an API key is missing

## Supported Modules

### Cortex Platform (14 content types)

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
| datasets | XQL dataset definitions (runtime/usage stats excluded) |
| content_packs | Installed content packs and their versions |
| rbac_roles | Role definitions and their permissions (names harvested from rbac_users) |
| rbac_user_groups | User group membership and source (names harvested from rbac_users) |
| attack_surface_rules | Attack surface management detection rules |

### Cortex Cloud AppSec (7 content types)

| Content Type | Description |
|--------------|-------------|
| applications | Application inventory and configuration |
| policies | Security policies for threat detection |
| rules | Custom security rules |
| repositories | Code repository configurations |
| integrations | Third-party integrations |
| application_configuration | Singleton application configuration |
| application_criteria | Application filtering criteria |

### Agent Configurations (10 content types)

Each Agent Configurations content type is a global singleton that produces exactly one `settings.yaml` file.

| Content Type | Description |
|--------------|-------------|
| content_management | Content update settings |
| agent_status | Agent status reporting configuration |
| auto_upgrade | Automatic agent upgrade settings |
| wildfire_analysis | WildFire analysis configuration |
| informative_btp_issues | Informative BTP issue settings |
| cortex_xdr_log_collection | XDR log collection configuration |
| action_center_expiration | Action Center expiration policy |
| critical_environment_versions | Critical environment version pinning |
| advanced_analysis | Advanced analysis configuration |
| endpoint_administration_cleanup | Endpoint administration cleanup policy |

### Cortex Cloud Workload Protection (CWP) (1 content type)

| Content Type | Description |
|--------------|-------------|
| policies | CWP policies (server-bumped createdAt/modifiedAt excluded for stable diffs) |

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

Running init against an instance that already has a config.toml is refused, so an existing
configuration cannot be overwritten by accident. Pass --force to replace it deliberately.

This creates the following structure:

```
production/
+-- .git/
+-- config.toml
+-- platform/
|   +-- dashboards/
|   +-- correlation_searches/
|   +-- biocs/
|   +-- widgets/
|   +-- authentication_settings/
|   +-- scripts/
|   +-- scheduled_queries/
|   +-- xql_library/
|   +-- rbac_users/
|   +-- datasets/
|   +-- rbac_roles/
|   +-- rbac_user_groups/
|   +-- attack_surface_rules/
+-- appsec/
|   +-- applications/
|   +-- policies/
|   +-- rules/
|   +-- repositories/
|   +-- integrations/
|   +-- application_configuration/
|   +-- application_criteria/
+-- agent/
|   +-- content_management/
|   +-- agent_status/
|   +-- auto_upgrade/
|   +-- wildfire_analysis/
|   +-- informative_btp_issues/
|   +-- cortex_xdr_log_collection/
|   +-- action_center_expiration/
|   +-- critical_environment_versions/
|   +-- advanced_analysis/
|   +-- endpoint_administration_cleanup/
+-- cwp/
    +-- policies/
```

Configure API access in production/config.toml:

```toml
[modules.platform]
enabled = true
fqdn = "api-production.xdr.eu.paloaltonetworks.com"
api_key = "${XSIAM_API_KEY}"
api_key_id = "${XSIAM_API_KEY_ID}"

[modules.appsec]
enabled = true
fqdn = "api-production.xdr.eu.paloaltonetworks.com"
api_key = "${APPSEC_API_KEY}"
api_key_id = "${APPSEC_API_KEY_ID}"

[modules.agent]
enabled = true
fqdn = "api-production.xdr.eu.paloaltonetworks.com"
api_key = "${AGENT_API_KEY}"
api_key_id = "${AGENT_API_KEY_ID}"

[modules.cwp]
enabled = true
fqdn = "api-production.xdr.eu.paloaltonetworks.com"
api_key = "${CWP_API_KEY}"
api_key_id = "${CWP_API_KEY_ID}"
```

Environment variables are expanded automatically using ${VARIABLE} syntax. gcgit also
recognises DEMISTO_BASE_URL, DEMISTO_API_KEY, and XSIAM_AUTH_ID as fallback variables
for cross-project compatibility.

Pull configurations:

```bash
gcgit platform pull --instance production
gcgit appsec pull --instance production
gcgit agent pull --instance production
gcgit cwp pull --instance production
```

All changes are automatically committed to the local Git repository.

A pull writes one file per object and removes local files for objects that no longer
exist on the platform, so the committed state matches the platform. Pruning only runs for
content types that pulled successfully: if an endpoint fails or returns a response gcgit
cannot interpret, the existing local files for that content type are left untouched and a
warning is printed.

## Commands

| Command | Description |
|---------|-------------|
| init --instance NAME | Create a new instance directory with module subdirectories |
| init --instance NAME --force | Replace the configuration of an existing instance |
| MODULE pull --instance NAME --strict | Exit non-zero if any content type fails to pull |
| MODULE pull --instance NAME --dry-run | Report what would change without writing or committing |
| MODULE pull --instance NAME --content-type TYPE | Pull only the named content types |
| MODULE pull --instance NAME --skip TYPE | Pull everything except the named content types |
| MODULE pull --instance NAME --no-git | Write files without staging or committing |
| MODULE pull --instance NAME --quiet | Report per content type rather than per file |
| platform pull --instance NAME | Pull all Cortex Platform configurations |
| MODULE,MODULE pull --instance NAME | Run the command against several modules in order |
| platform diff --instance NAME | Show differences between local and remote |
| platform test --instance NAME | Test API connectivity to the Cortex Platform module |
| appsec pull --instance NAME | Pull all AppSec configurations from the platform |
| appsec diff --instance NAME | Show differences between local and remote |
| appsec test --instance NAME | Test API connectivity to the AppSec module |
| agent pull --instance NAME | Pull all Agent Configurations singletons from the platform |
| agent diff --instance NAME | Show differences between local and remote |
| agent test --instance NAME | Test API connectivity to the Agent module |
| cwp pull --instance NAME | Pull all CWP configurations from the platform |
| cwp diff --instance NAME | Show differences between local and remote |
| cwp test --instance NAME | Test API connectivity to the CWP module |

Each module supports the same set of operations (pull, diff, test) through a consistent interface.

### Leaving content types out

`--skip` excludes content types, which is useful when one of them is large or noisy and
the rest are wanted:

```bash
gcgit platform pull --instance production --skip attack_surface_rules,datasets
```

Both `--skip` and `--content-type` accept a comma separated list, values separated by
spaces, or repetition of the flag. These are equivalent:

```bash
--skip attack_surface_rules,datasets
--skip attack_surface_rules, datasets
--skip attack_surface_rules datasets
--skip attack_surface_rules --skip datasets
```

A skipped name the module does not have is ignored, so one list can be used across a run
covering several modules. A name no module has is rejected before any work starts.

Two modules can have a content type of the same name: both AppSec and CWP have `policies`.
Qualify the name with a module to skip it in one place only, written the way the
repository already stores it:

```bash
gcgit platform,appsec,cwp pull --instance production --skip cwp/policies
```

That leaves the AppSec policies alone. A path copied out of the working tree is a valid
argument, since the separator is the same one the directory layout uses.

### Quieter output

A pull prints a line per file, which on a large content type runs to hundreds of lines.
Pass `--quiet` (or `-q`) to report per content type instead:

```
Pulling xql_library...
  Found 422 xql_library(s)
  Removed 2 stale file(s)
```

Counts, warnings and errors are still shown. Removals are reported as a count rather than
hidden, because deleting a file is the only destructive thing a pull does.

### Running several modules at once

Give a comma-separated list in place of a single module name:

```bash
gcgit platform,appsec,agent pull --instance production
gcgit platform,cwp diff --instance production
```

The modules run in the order given. A module that fails does not stop the ones after it,
because a scheduled backup should capture what it can rather than abandon everything over
one unavailable endpoint. The run ends with a summary naming any that failed:

```
2 of 3 module(s) completed.
Failed: cwp
```

The exit code is still zero in that case, matching how a single pull treats a failing
content type. Pass `--strict` to make any failure a non-zero exit, which is what a
scheduled job should use.

An unknown module name is rejected before any work starts, so a typo cannot leave some
modules pulled and others not.

The test command verifies connectivity against the platform health check endpoint and
treats any non-success response as a failure, reporting 401, 402 and 403 distinctly.

The diff command retrieves each content type once rather than once per local file, and
reports three outcomes: DIFF where a stored object differs from the platform, LOCAL ONLY
where a stored object has no platform counterpart, and REMOTE ONLY where the platform has
an object that has not been stored. A content type that cannot be retrieved is reported as
an error rather than being assumed absent.

### Development Status

| Status | Operations |
|--------|------------|
| Available | pull, diff, test, init, status, validate |
| Not implemented | push, delete, deploy |

gcgit reads from a Cortex platform and never writes to one. The push, delete and deploy
commands are hidden from help and exit with an error if invoked.

## Configuration

Each instance has a config.toml file with per-module credential blocks:

```toml
[modules.platform]
enabled = true
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "${XSIAM_API_KEY}"
api_key_id = "${XSIAM_API_KEY_ID}"

[modules.appsec]
enabled = false
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "${APPSEC_API_KEY}"
api_key_id = "${APPSEC_API_KEY_ID}"

[modules.agent]
enabled = false
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "${AGENT_API_KEY}"
api_key_id = "${AGENT_API_KEY_ID}"

[modules.cwp]
enabled = false
fqdn = "api-instance.xdr.region.paloaltonetworks.com"
api_key = "${CWP_API_KEY}"
api_key_id = "${CWP_API_KEY_ID}"
```

Set enabled = false to disable a module whilst keeping its configuration. Each module can use different API credentials and even different platform FQDNs.

### Plain HTTP

Each module block accepts an optional force_http key, written as force_http = false by
gcgit init:

```toml
[modules.platform]
enabled = true
fqdn = "localhost:8080"
api_key = "${XSIAM_API_KEY}"
api_key_id = "${XSIAM_API_KEY_ID}"
force_http = true
```

When set, requests for that connection are sent over HTTP instead of HTTPS. This exists for
pointing gcgit at a local mock or a development endpoint that does not terminate TLS.

The API key and key ID travel unencrypted when it is enabled, so gcgit prints a warning on
every run and refuses to use an HTTP proxy for that connection. Do not enable it against a
production tenant. There is deliberately no environment variable for this setting: the
config file is the only place it can be turned on.

Note that a scheme written into fqdn is stripped rather than honoured. force_http is the
only thing that selects HTTP.

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

Each YAML file contains the complete configuration for one object. Changes to individual objects produce clean, readable Git diffs. Singleton content types (Agent Configurations, AppSec `application_configuration`) produce a single `settings.yaml` file per content type.

## Building

Requirements:

- Rust 1.70 or later
- Cortex XSIAM, Cortex Cloud AppSec, Agent Configurations, and/or Cortex Cloud Workload Protection API access
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
gcgit platform --help
gcgit appsec --help
gcgit agent --help
gcgit cwp --help
```

## Automated backups in CI

gcgit writes into the repository that already contains the instance directory. If the
instance sits inside a checkout, gcgit uses that checkout rather than creating a nested
repository, so the pulled files are tracked by the repository the workflow pushes.

A scheduled GitHub Actions job needs no configuration file in the repository. The instance
config holds only variable references, so it can be regenerated on each run and the
credentials come from repository secrets:

```yaml
name: Back up Cortex configuration
on:
  schedule:
    - cron: "0 * * * *"
  workflow_dispatch:

permissions:
  contents: write

jobs:
  backup:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install gcgit
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          VERSION=2.6.1
          # Release assets on a private repository require authentication, so this
          # uses the GitHub CLI rather than a plain download. For a public repository
          # a curl of the same URL works without a token.
          gh release download "v${VERSION}" \
            --repo "${{ github.repository }}" \
            --pattern "gcgit-${VERSION}-linux-musl-x86_64.tar.gz"
          tar -xzf "gcgit-${VERSION}-linux-musl-x86_64.tar.gz"
          sudo install -m 0755 "gcgit-${VERSION}-linux-musl-x86_64" /usr/local/bin/gcgit

      - name: Regenerate the instance configuration
        run: gcgit init --instance production --force

      - name: Pull
        env:
          XSIAM_FQDN: ${{ secrets.XSIAM_FQDN }}
          XSIAM_API_KEY: ${{ secrets.XSIAM_API_KEY }}
          XSIAM_API_KEY_ID: ${{ secrets.XSIAM_API_KEY_ID }}
        run: |
          for module in platform appsec agent cwp; do
            gcgit "$module" pull --instance production --strict
          done

      - name: Push
        run: |
          git config user.name "gcgit"
          git config user.email "gcgit@users.noreply.github.com"
          git push
```

`init --force` rewrites only config.toml and the directory tree; it does not touch pulled
files. The generated config.toml contains `${XSIAM_FQDN}` style references rather than
values, and the instance .gitignore keeps it out of the repository, so no secret is ever
written to a tracked file.

gcgit stages and commits the files it wrote. To let the workflow own the commit instead,
pass `--no-git`: files are written and nothing is staged or committed.

Use `--strict` so the job fails when a content type cannot be retrieved, rather than
committing a partial backup silently.

## The platform module was previously called xsiam

The module covering dashboards, correlation rules, BIOCs, widgets, scripts, the XQL
library, RBAC, datasets and content packs is now called `platform`. It was called `xsiam`,
which was inaccurate once the same endpoints served Cortex Cloud.

Nothing needs to change in an existing installation:

- `gcgit xsiam ...` still works. It is an alias for `gcgit platform ...`, kept for scripts
  and pipelines, and hidden from help.
- A config file with a `[modules.xsiam]` section is still read. `[modules.platform]` is
  preferred where both are present, and is what `gcgit init` now generates.
- An instance whose files are already under `xsiam/` keeps using that directory, and says
  so on each run. Only a new instance gets a `platform/` directory.

To move an existing instance across, rename the directory and the config section together:

```bash
git mv production/xsiam production/platform
sed -i 's/^\[modules.xsiam\]/[modules.platform]/' production/config.toml
```

## Change Log

See [CHANGELOG.md](CHANGELOG.md) for observable changes by release, including the
behaviour changes to expect when upgrading.

## Licence

This project is licensed under the GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later). See the [LICENSE](LICENSE) file for the full licence text.
