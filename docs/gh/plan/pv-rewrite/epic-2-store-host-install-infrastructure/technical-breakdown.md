# Technical Breakdown: Epic 2 - Store, Host, And Install Infrastructure

## Module Roles

| Module | Responsibility |
| --- | --- |
| `internal/host` | Canonical pv path families and layout validation. |
| `internal/store` or `internal/control` | Store schema version, applied migrations, and migration runner seam. |
| `internal/installer` | Plan graph, resolver/downloader/installer adapters, execution results, and shim writer. |
| `internal/control` | Reconcile signal seam used after durable persistence. |
| `docs/gh/plan/pv-rewrite` | Contract-version decision and MVP scope guardrails. |

## Canonical Path Families

| Family | Shape |
| --- | --- |
| Shims | `~/.pv/bin` |
| PHP runtimes | `~/.pv/runtimes/php/<version>` |
| Tools | `~/.pv/tools/<name>/<version>` |
| Service binaries | `~/.pv/services/<name>/<version>/bin` |
| Stateful data | `~/.pv/data/<name>/<version>` |
| Logs | `~/.pv/logs/<name>/<version>.log` |
| Store | `~/.pv/state/pv.db` |
| Artifact cache | `~/.pv/cache/artifacts` |
| Config | `~/.pv/config` |

Rules:

- `bin` contains shims or symlinks only.
- Resource names and versions are validated path segments.
- Data paths never live under runtime, tool, or service binary roots.
- New path families require a planning update before implementation.

## Store Migration Shape

1. Store exposes current schema version.
2. Store records applied migration IDs in order.
3. Migration runner enumerates pending migrations.
4. Migrations are forward-only.
5. Migration failure returns an error and does not reinterpret state.

Checksum/integrity metadata is explicitly deferred to `post-mvp-backlog.md` and is not an Epic 2 implementation task.

## Install Planner Shape

1. Plan items include kind (`runtime`, `tool`, `service`), name, and version.
2. Plan validation rejects duplicate identities and missing dependencies.
3. Topological ordering is deterministic.
4. Downloads run through a bounded scheduler.
5. Installs execute in dependency order.
6. Failed prerequisites skip dependent work.
7. Shims are exposed only after install success.
8. Desired state persistence happens after durable install work.
9. Daemon signal seam is called only after persistence succeeds.

## Contract Version Decision

Epic 2 records the decision, and Epic 4 implements it:

- `pv.yml` includes top-level `version: 1`.
- Epic 2 does not parse the full project contract.
- Issue #133 is complete when the decision is documented and linked to #171.

## Non-Goals

- No real runtime, tool, or service installers.
- No daemon implementation.
- No Laravel project parsing beyond the contract-version decision.
- No expensive artifact workflows.
