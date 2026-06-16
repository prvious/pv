# PV Release Candidate Checklist

Use this checklist for PV-125 release candidate validation. The current RC target is Apple Silicon/staging RC. The full public v1 native artifact matrix remains `darwin-arm64,darwin-amd64`; the conditional `darwin-amd64` gate is held until StaticPHP v3/macOS x86_64 validation is stable enough.

## Evidence

| Field | Value |
| ----- | ----- |
| RC version |  |
| Commit |  |
| Tester |  |
| macOS version |  |
| Architecture |  |
| Target scope | Apple Silicon/staging RC or full public v1 |
| Installer URL |  |
| App manifest URL |  |
| Artifact manifest URL |  |
| Workflow run IDs |  |

Default-track matrix under validation: PHP/FrankenPHP `8.5`, Composer `2`, MySQL `8.4`, Postgres `18`, Redis `8.8`, Mailpit `1`, and RustFS `1`.

## Artifact Publication

| Check | Evidence |
| ----- | -------- |
| Artifact Recipes workflow run completed for the target scope. |  |
| Artifact Publication workflow published immutable archives and records. |  |
| Stable artifact manifest URL returns the expected default tracks. |  |
| Apple Silicon/staging RC used `darwin-arm64` validation. |  |
| Full public v1, when enabled, used `darwin-arm64,darwin-amd64` validation. |  |
| Any deferred `darwin-amd64` lane is recorded as an intentional conditional gate. |  |

## Fresh Install

| Check | Evidence |
| ----- | -------- |
| Installer downloads the RC PV binary and verifies checksum. |  |
| Active symlink points at `~/.pv/bin/releases/<version>/pv`. |  |
| Installer shell profile behavior matches selected flags. |  |
| Installer runs setup unless `--no-setup` is selected. |  |

## Setup And System

| Check | Evidence |
| ----- | -------- |
| `pv setup` fetches the artifact manifest before default-resource planning. |  |
| Cached manifest fallback warning appears only when a cache exists. |  |
| DNS resolver config is PV-owned and `.test` resolves locally. |  |
| `pf` redirects route loopback `80` and `443` to Gateway high ports. |  |
| PV local CA is trusted in the macOS System keychain. |  |
| LaunchAgent is registered, daemon starts, and system reconciliation installs desired default setup resources. |  |

## Project Flow

| Check | Evidence |
| ----- | -------- |
| `pv link` records a Project and requests reconciliation. |  |
| `pv open` opens the primary Project hostname. |  |
| `pv list` reports Project, PHP, serving, resource, and env status. |  |
| `pv.yml` with YAML anchors is accepted after YAML merge resolution. |  |
| `pv project:env` renders expected values without mutating `.env`. |  |

## Managed Resources

| Check | Evidence |
| ----- | -------- |
| PHP/FrankenPHP `8.5` installs as a matched pair. |  |
| Composer `2` runs through the PV PHP shim. |  |
| MySQL `8.4` starts, passes readiness, and allocates databases. |  |
| Postgres `18` starts, passes readiness, and allocates databases. |  |
| Redis `8.8` starts, passes readiness, and renders prefixes. |  |
| Mailpit `1` starts and `pv mailpit:open` or `pv mail:open` targets the UI. |  |
| RustFS `1` starts, passes readiness, and `pv rustfs:open` or `pv s3:open` targets the console. |  |

## Update

| Check | Evidence |
| ----- | -------- |
| `pv update --check` reports PV app and installed Managed Resource availability. |  |
| `pv update --check --json` returns valid JSON. |  |
| `pv update` self-updates the PV application before Managed Resource updates when an app update is available. |  |
| Managed Resource update phase updates only installed tracks. |  |
| Daemon restart/reconnect evidence is captured. |  |

## Diagnostics

| Check | Evidence |
| ----- | -------- |
| `pv status` reports daemon, Gateway, DNS, ports, CA, Projects, and Managed Resources. |  |
| `pv status --json` returns valid JSON without secrets. |  |
| `pv doctor` reports focused repair commands for failures. |  |
| `pv logs --all` includes PV-owned log streams. |  |
| `pv jobs --json` includes recent job status. |  |

## Uninstall

| Check | Evidence |
| ----- | -------- |
| `pv uninstall` removes PV system integrations and app binaries while preserving data. |  |
| `pv uninstall --prune --force` removes PV-owned state under `~/.pv`. |  |
| Project directories and user-owned files are not deleted. |  |
| Shell profile backup evidence is captured when a PV-managed block is removed. |  |

## Blockers

| Blocker | Owner | Decision |
| ------- | ----- | -------- |
|  |  |  |

## Sign-Off

| Role | Name | Date | Notes |
| ---- | ---- | ---- | ----- |
| Release tester |  |  |  |
| Engineering reviewer |  |  |  |
| Release owner |  |  |  |
