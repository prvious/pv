# Technical Breakdown: Laravel-First Local Control Plane

## Module Roles

The package names below are the planned rewrite package roles and should stay
stable unless the architecture document is updated first.

| Module | Responsibility |
| --- | --- |
| `internal/app` | Command use cases and orchestration between CLI and domain modules. |
| `internal/console` | Human output, prompts, tables, stderr/stdout discipline. |
| `internal/control` | Desired-state model, observed-status model, reconcile orchestration. |
| `internal/store` | SQLite persistence target, locks, schema versioning, migrations. |
| `internal/host` | OS primitives: paths, launchd/systemd, signals, processes, DNS, TLS. |
| `internal/catalog` | Supported versions and artifact metadata. |
| `internal/installer` | Downloads, cache, extraction, atomic swaps, install planner. |
| `internal/supervisor` | Resource-agnostic process runner. |
| `internal/resources/php` | PHP runtime installation, version resolution, shims. |
| `internal/resources/composer` | Composer install and PHP-runtime dependency. |
| `internal/resources/mago` | Mago tool install and shim behavior. |
| `internal/resources/laravel` | Project contract resolution and Laravel-specific setup surface. |
| `internal/resources/gateway` | `.test` routing, aliases, DNS, TLS, FrankenPHP process definitions. |
| `internal/resources/postgres` | Postgres version lines, data ownership, process and DB commands. |
| `internal/resources/mysql` | MySQL version lines, init, sockets, privileges, process and DB commands. |
| `internal/resources/redis` | Redis version lines, process flags, persistence, env values. |
| `internal/resources/mailpit` | Mailpit SMTP/web behavior, ports, env values, process definitions. |
| `internal/resources/rustfs` | RustFS S3 credentials, ports, routes, data, env values. |

## Data Authority

| Data | Authority | Format |
| --- | --- | --- |
| Human-authored project contract | Project repository | YAML |
| Machine-owned desired state | `pv` store | SQLite target |
| Machine-owned observed status | `pv` store | SQLite target |
| Runtime/service/tool metadata | `pv` store | SQLite target |
| Remote artifact metadata | Generated artifact manifest | JSON |
| Logs | Filesystem | Text logs |

File-backed state is acceptable only as early scaffold. It should not grow into
the real concurrency or migration model.

## Command Flow

1. Parse flags and args.
2. Load project or global context.
3. Validate requested change.
4. Write desired state.
5. Signal daemon if daemon-observed state changed.
6. Print a human-readable result to stderr, unless the command is explicitly
   pipeable.

Commands must not spawn long-running resource processes directly, hand-roll
install flows, mutate multiple state files independently, generate gateway
config outside the gateway controller, or write observed status.

## Reconcile Flow

1. Load desired state.
2. Let controllers inspect resource families.
3. Perform install/config/process/routing work needed to match desired state.
4. Submit process definitions to the supervisor.
5. Write observed status.
6. Surface failures through `pv status`.

## Capability Model

Do not create a shallow generic service abstraction. Use capability vocabulary
to describe resource behavior, but introduce interfaces only when there are
real adapters and real leverage.

| Resource | Capabilities |
| --- | --- |
| PHP | Installable, HasCliShim |
| Composer | Installable, HasCliShim, DependsOnRuntime |
| Mago | Installable, HasCliShim |
| Gateway | Runnable |
| Postgres | Installable, Runnable, Stateful, ExposesEnv, HasDatabaseCommands |
| MySQL | Installable, Runnable, Stateful, ExposesEnv, HasDatabaseCommands |
| Redis | Installable, Runnable, Stateful, ExposesEnv |
| Mailpit | Installable, Runnable, ExposesEnv, HasHttpConsole |
| RustFS | Installable, Runnable, Stateful, ExposesEnv, HasHttpConsole |

## Filesystem Layout

Target shape:

```text
~/.pv/
  bin/
  runtimes/php/<version>/
  tools/composer/<version>/
  tools/mago/<version>/
  services/postgres/<version>/bin/
  services/mysql/<version>/bin/
  services/redis/<version>/bin/
  services/mailpit/<version>/bin/
  services/rustfs/<version>/bin/
  data/<resource>/<version>/
  logs/<resource>/<version>.log
  state/pv.db
  cache/artifacts/
  config/
```

Rules:

- `bin/` is for shims and symlinks only.
- Real binaries live under runtime, tool, or service roots.
- Stateful data lives under `data/`.
- Logs live under `logs/`.
- Machine state lives under `state/`.
- Downloaded artifacts live under `cache/artifacts/`.
- No resource invents a path family without an architecture decision.

## Major Risks

- Flat issues encourage partial slices that skip implementation plans.
- Capability interfaces can become fake sameness if introduced too early.
- File-backed state can become accidental architecture.
- Gateway/DNS/TLS work can drag privileged OS behavior into unit tests.
- `pv link` can regress into hidden Laravel magic if project contract ownership
  is not explicit.
- Service lifecycle helpers can erase real differences between databases, mail,
  object storage, cache, and gateway infrastructure.
