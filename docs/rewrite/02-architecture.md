# pv Rewrite Architecture: Desired-State Control Plane

## Purpose

This document records the architecture behind the Laravel-first rewrite PRD.

The target is:

> `pv` is a local desired-state control plane for Laravel development.

The rewrite is not about changing languages. Go remains the right fit because `pv` is OS orchestration: processes, files, ports, downloads, signals, launchd/systemd, DNS, TLS, state files, daemon behavior, and cross-platform release builds.

The real problem is authority. The current prototype has several partial centers of truth: project config, registry JSON, state JSON, config paths, service packages, command wrappers, setup automation, the daemon manager, watcher, and supervisor. Too many modules know what should exist.

The rewrite gives that responsibility one shape.

## Architecture Rule

```text
Commands do not do work. They request state changes.
Controllers do work.
The supervisor only runs processes.
The store is the authority.
Resources expose capabilities, not fake sameness.
Laravel is the primary product path.
```

## Simplicity Rule

The rewrite should remove anything that is unnecessary or likely to cause friction.

Every dependency, wrapper, command surface, prompt, styling layer, abstraction, background process, and storage mechanism must pass this test:

```text
Does this make pv easier to understand, operate, test, and maintain than the smallest direct implementation?
```

If the answer is not clearly yes, leave it out.

This is especially important in the CLI layer. A polished command should not require a heavy presentation stack. The default should be simple commands, clear text, stable exit codes, and predictable state changes.

## Control-Plane Model

The architecture has five core concepts.

### Resources

Resources are things `pv` manages:

- Laravel project
- PHP runtime
- Composer
- Mago
- web gateway
- Postgres
- MySQL
- Redis
- Mailpit
- RustFS

A resource is not necessarily a service. PHP is a runtime. Composer is a tool. FrankenPHP is gateway infrastructure. Postgres is a stateful database. Mailpit is a runnable mail catcher with an HTTP console.

### Desired State Store

The desired-state store is the mutable authority for machine-owned state.

Commands validate user intent and write desired state. They do not directly install services, mutate routing, spawn long-running processes, or reconcile the machine.

Desired state answers:

- Which projects should be linked?
- Which project contract version was resolved?
- Which runtime and service versions should exist?
- Which resources should be running?
- Which web routes should exist?
- Which tools should be exposed?

### Controllers

Controllers reconcile one resource family at a time. They read desired state, inspect reality, perform the needed work, and write observed status.

Examples:

- The PHP runtime controller ensures requested PHP versions exist and shims point at the right runtime behavior.
- The Laravel project controller resolves project contracts and setup requirements.
- The gateway controller renders routes, certificates, DNS, and FrankenPHP process definitions.
- Database controllers install version lines, initialize data directories, expose env values, and build process definitions.
- Tool controllers install and expose Composer or Mago.

Controllers may coordinate through the store, but they should not become a central generic service framework.

### Supervisor

The supervisor is a dumb process runner.

It owns:

- start
- stop
- readiness checks
- crash restart
- restart budget
- log file attachment
- process liveness

It does not know what Laravel, PHP, Postgres, Redis, Mailpit, RustFS, Composer, Caddy, or FrankenPHP mean. Controllers translate resource state into supervisor process definitions.

### Observed Status

Observed status records what happened when controllers reconciled desired state.

Status should include, where relevant:

- desired state
- observed state
- process ID
- port
- host
- log path
- last error
- last reconcile time
- next action

Observed status must be separate from desired state. A crash should not corrupt what the user asked for.

## Sources Of Truth

Use each storage format for one job.

| Data | Authority | Format |
| --- | --- | --- |
| Human-authored project contract | Project repository | YAML |
| Machine-owned desired state | `pv` store | SQLite |
| Machine-owned observed status | `pv` store | SQLite |
| Runtime/service/tool metadata | `pv` store | SQLite |
| Remote artifact metadata | Generated artifact manifest | JSON |
| Logs | Filesystem | Text logs |

`pv.yml` remains the project-level contract. It is reviewed and committed by humans.

SQLite should become the machine-owned mutable store. It replaces the growing split between registry JSON, state JSON, status files, and ad hoc metadata files.

Before GA, state can break freely. At GA, the store should introduce:

- `state_schema_version`
- `contract_version`
- `applied_migrations`

Migrations should be forward-only and recorded in the store. Destructive migrations should snapshot affected files first when possible.

## Capability-Based Resources

Do not force resources under one shallow `Service` abstraction. Model resources by capabilities.

Useful capabilities:

- `Installable`
- `Runnable`
- `Stateful`
- `ExposesEnv`
- `HasDatabaseCommands`
- `HasHttpConsole`
- `HasCliShim`
- `DependsOnRuntime`

Example mapping:

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

Capability interfaces should stay narrow. A resource should not implement methods it cannot honestly support.

## Gateway Model

FrankenPHP should be treated as gateway infrastructure for web projects, not as a user-managed backing service.

The gateway controller owns:

- `.test` host routing
- aliases
- Caddy configuration
- local TLS certificates
- DNS integration
- FrankenPHP process definitions
- per-PHP-version serving behavior

Laravel developers should experience this through project commands, not through a `frankenphp:*` service surface.

## Laravel-First Product Path

The polished path is Laravel.

MVP command direction:

```text
pv init
pv link
pv open
pv artisan migrate
pv db
pv mail
pv s3
```

Generic PHP and static sites can exist as manual or advanced paths later. They should not shape the MVP architecture.

Laravel Octane is out of scope for MVP.

## CLI UX

The default UX should be scriptable first and polished second.

Good default output:

```text
✓ PHP 8.4 installed
✓ Composer installed
✓ Daemon enabled
✓ Laravel app linked at https://acme.test
```

`pv setup` should be a thin guided wrapper around real commands and desired-state changes. It should not become a separate TUI-only product path.

Production commands should return errors, use stderr for status output, and reserve stdout for pipeable output.

Fang is not part of the locked rewrite architecture. It belongs to the prototype unless a later decision explicitly chooses it again for a specific reason. It is likely unnecessary friction for the rewrite's command model. The rewrite should start with the smallest command layer that satisfies:

- predictable help;
- useful errors;
- stable exit codes;
- clean stdout for pipeable commands;
- stderr for human status;
- testable command construction.

The rewrite may still use Cobra if it remains the pragmatic command parser, but command presentation should not be delegated to Fang by default.

## Install Planner

Install and update flows should use a shared planner instead of command-specific glue.

The planner takes a validated plan:

```text
php 8.4
composer
mago
postgres 18
```

Then runs phases:

1. Resolve artifacts and dependencies.
2. Download artifacts in bounded parallelism.
3. Install in dependency order.
4. Expose shims atomically.
5. Persist desired state.
6. Signal daemon reconciliation.

This belongs in installer/control-plane infrastructure. It should not be duplicated inside PHP, Mago, Composer, and each service.

## Filesystem Layout

Do not let `~/.pv` grow organically.

Target shape:

```text
~/.pv/
  bin/                         # user PATH shims only
  runtimes/
    php/
      8.4/
  tools/
    composer/
    mago/
  services/
    postgres/
      18/
        bin/
    mysql/
      8.4/
        bin/
    redis/
      8.6/
        bin/
    mailpit/
      1/
        bin/
    rustfs/
      1.0.0-beta/
        bin/
  data/
    postgres/
      18/
    mysql/
      8.4/
    redis/
      8.6/
    mailpit/
      1/
    rustfs/
      1.0.0-beta/
  logs/
    postgres/
      18.log
    mysql/
      8.4.log
    redis/
      8.6.log
    mailpit/
      1.log
    rustfs/
      1.0.0-beta.log
  state/
    pv.db
  cache/
    artifacts/
  config/
```

Rules:

- `bin/` is for shims and symlinks only.
- Real binaries live under runtime, tool, or service roots.
- Stateful data lives under `data/`.
- Logs live under `logs/`.
- Machine state lives under `state/`.
- Downloaded artifacts live under `cache/artifacts/`.
- No resource should invent a special path family without an explicit architecture decision.

## Repository Layout During Rewrite

Before new rewrite code is introduced at the repository root, move the current prototype implementation into:

```text
legacy/
  prototype/
```

The prototype should remain buildable and testable from that directory. It is a reference implementation, not the active architecture.

Move the existing Go application as a complete module:

- current `go.mod` and `go.sum`;
- current `main.go`;
- current command and internal implementation packages;
- old binary-building scripts and release configuration that belong to the prototype;
- any prototype-specific README or usage documentation that would otherwise describe the old root layout.

Keep repository coordination and rewrite materials at the root:

- agent instructions;
- issue-tracker and domain docs;
- rewrite PRD and architecture docs;
- repository ignore/config files that still apply globally.

After the move, the root can host the new rewrite module without colliding with old package names such as `cmd/` and `internal/`.

Rules:

- The old module may keep the original module path for reference buildability.
- The new root module becomes the active product once it is scaffolded.
- Do not import old prototype packages from the new rewrite.
- Copy behavior or tests from the prototype deliberately; do not create shared code between prototype and rewrite.
- Mark the prototype clearly in docs so agents do not treat it as the active implementation.

## Package Roles

The exact package names can change during implementation, but the role boundaries should be clear.

Possible layout:

```text
internal/
  app/          # command use cases
  console/      # terminal output, prompts, tables
  control/      # desired-state model and reconcile orchestration
  store/        # SQLite persistence, locks, migrations
  host/         # OS, launchd/systemd, signals, paths, process primitives
  catalog/      # supported versions and artifact metadata
  installer/    # downloads, cache, extraction, atomic swaps, install planner
  supervisor/   # process runner
  resources/
    php/
    composer/
    mago/
    laravel/
    gateway/
    postgres/
    mysql/
    redis/
    mailpit/
    rustfs/
```

The point is role-based organization, not the literal names. `internal/` is useful in Go. The problem to avoid is an `internal/` junk drawer where package names encode history instead of responsibility.

## Command Flow

Commands follow a consistent flow:

1. Parse flags and arguments.
2. Load project or global context.
3. Validate the requested change.
4. Write desired state.
5. Signal the daemon when daemon-observed state changed.
6. Print a human-readable result.

Commands should not:

- spawn long-running resource processes directly;
- hand-roll install flows;
- mutate several state files independently;
- generate gateway config outside the gateway controller;
- update observed status directly.

## Reconcile Flow

The daemon reconcile loop follows a consistent flow:

1. Load desired state.
2. Let each controller inspect its resource family.
3. Run install/config/process/routing work needed to match desired state.
4. Submit process definitions to the supervisor.
5. Write observed status.
6. Surface failures in `pv status`.

Controllers should be independently testable with fake host primitives and temporary stores.

## Status Model

Status output should explain:

- what is desired;
- what is running;
- what failed;
- what `pv` tried;
- where logs are;
- what the user can do next.

Status is a product surface, not just debug output. It should be designed as carefully as command behavior.

## Migration Model

Before GA, breaking state changes are allowed.

At GA, introduce state and filesystem migrations inspired by database migration systems:

```text
store/migrations/
  0001_initial
  0002_service_paths
  0003_runtime_layout
```

Migration rules:

- forward-only;
- recorded in SQLite;
- unique version per migration;
- checksum or equivalent integrity check;
- idempotent where possible;
- snapshot before destructive file moves when possible;
- no hidden upgrades that silently reinterpret state.

`pv.yml` should also carry a contract version once the contract stabilizes.

## What Not To Abstract

Keep these behaviors explicit:

- Postgres initialization, HBA/config behavior, sockets, and database commands.
- MySQL initialization, socket/PID behavior, privileges, and database commands.
- Redis process flags and persistence behavior.
- Mailpit SMTP/web ports and mail env variables.
- RustFS credentials, API/console ports, routes, and S3 env variables.
- Gateway routing, TLS, DNS, and per-version PHP serving.
- Composer's dependency on PHP runtime resolution.

Shared helpers should remove repeated mechanics, not erase resource identity.

## Open Decisions

These are intentionally left for implementation planning:

- Exact SQLite schema.
- Whether the rewrite happens in-place or through a branch-by-branch strangler migration.
- Exact package names.
- Exact `pv.yml` contract version field.
- Whether generic PHP/static support stays in the first rewrite branch as manual advanced support or moves fully post-MVP.
- The exact Laravel helper command surface beyond `pv init`, `pv link`, `pv open`, and project-aware Artisan/database/mail/S3 shortcuts.
