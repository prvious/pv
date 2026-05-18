# Issues Checklist: Epic 3 - Runtime, Daemon, And Resources

Create these issues after Epic 2 is published.

## Published Issues

Milestone: `pv rewrite MVP`

| Type | Issue | Title |
| --- | --- | --- |
| Epic | #142 | Epic: Runtime, Daemon, And Resources |
| Feature | #143 | Feature: PHP Runtime And Composer Tooling |
| Feature | #144 | Feature: Daemon And Supervisor With Mailpit |
| Feature | #145 | Feature: Stateful Database Resources |
| Feature | #146 | Feature: Cache, Mail, And Object Storage Resources |
| Enabler | #147 | Enabler: Add PHP Runtime Desired-State Controller |
| User Story | #148 | User Story: Request PHP Runtime Install Desired State |
| Enabler | #149 | Enabler: Add Composer Tool Dependency On PHP Runtime |
| User Story | #150 | User Story: Expose Runtime-Aware CLI Shims Atomically |
| Test | #151 | Test: PHP And Composer Runtime Dependency Behavior |
| Enabler | #152 | Enabler: Add Daemon Reconcile Loop |
| Enabler | #153 | Enabler: Add Resource-Agnostic Supervisor |
| User Story | #154 | User Story: Manage Mailpit As First Runnable Resource |
| User Story | #155 | User Story: Record Runnable Resource Observed Status |
| Test | #156 | Test: Daemon And Supervisor Process Lifecycle |
| User Story | #157 | User Story: Manage Postgres Version-Line Resource |
| User Story | #158 | User Story: Expose Postgres Env Values And Explicit Database Commands |
| User Story | #159 | User Story: Manage MySQL Version-Line Resource |
| User Story | #160 | User Story: Preserve MySQL-Specific Initialization And Process Semantics |
| Test | #161 | Test: Stateful Database Resource Behavior |
| User Story | #162 | User Story: Manage Redis Runnable Stateful Resource |
| User Story | #163 | User Story: Manage RustFS S3 Resource |
| User Story | #164 | User Story: Expose Cache, Mail, And Object Storage Env Values |
| Test | #165 | Test: Redis, Mailpit, And RustFS Resource Behavior |

Tracker hygiene performed:

- Legacy flat issues #100-#105 remain reference-only.
- PR #115 remains reference-only.
- Added superseded/reference comments to #100-#105 and #115.
- Added `ready-for-agent` to Epic 3 leaf issues #147-#165.

## Epic Issue

### Title

`Epic: Runtime, Daemon, And Resources`

### Labels

`epic`, `priority-critical`, `value-high`, `runtime`, `resource`, `control-plane`

### Body

```markdown
## Epic Description

Build the managed runtime, daemon, supervisor, and resource layer behind the
Laravel-first pv rewrite.

Legacy references: #100, #101, #102, #103, #104, #105, #115.

## Business Value

- Laravel projects can use pv-managed PHP and Composer.
- Long-running resources reconcile through a daemon and supervisor model.
- Mailpit, Postgres, MySQL, Redis, and RustFS become explicit declared
  resources with predictable status and env values.

## Features

- [ ] Feature: PHP Runtime And Composer Tooling
- [ ] Feature: Daemon And Supervisor With Mailpit
- [ ] Feature: Stateful Database Resources
- [ ] Feature: Cache, Mail, And Object Storage Resources

## Acceptance Criteria

- [ ] PHP runtime desired state can be requested, reconciled, and reported.
- [ ] Composer depends on a PHP runtime and reports blocked status when missing.
- [ ] Runtime-aware CLI shims are exposed atomically.
- [ ] Daemon reconcile loop responds to durable desired-state changes.
- [ ] Supervisor remains resource-agnostic.
- [ ] Mailpit runs as the first supervised runnable resource.
- [ ] Postgres and MySQL are explicit stateful database resources.
- [ ] Redis is a runnable stateful cache resource.
- [ ] RustFS is an S3 resource with redacted credential status.
- [ ] Resource env values are explicit and do not infer services from `.env`.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Supervisor resource-agnostic checks pass.
- [ ] Root verification passes.
- [ ] No expensive artifact workflows were run unless explicitly requested.
```

## Feature Issues

### Feature: PHP Runtime And Composer Tooling

**Labels:** `feature`, `priority-critical`, `value-high`, `runtime`, `control-plane`

```markdown
## Feature Description

Add managed PHP runtime and Composer tooling behavior using desired state,
controllers, install planning, and atomic runtime-aware shims.

## Parent Epic

Epic: Runtime, Daemon, And Resources

## Stories And Enablers

- [ ] Enabler: Add PHP Runtime Desired-State Controller
- [ ] User Story: Request PHP Runtime Install Desired State
- [ ] Enabler: Add Composer Tool Dependency On PHP Runtime
- [ ] User Story: Expose Runtime-Aware CLI Shims Atomically
- [ ] Test: PHP And Composer Runtime Dependency Behavior

## Dependencies

Blocked by:

- Epic 2: Store, Host, And Install Infrastructure

Blocks:

- Epic 4 project init, link, gateway, setup, and helper commands

## Acceptance Criteria

- [ ] PHP runtime desired state is persisted.
- [ ] PHP runtime controller reconciles through canonical runtime paths.
- [ ] Composer desired state records required PHP runtime.
- [ ] Composer reports blocked status when the required runtime is missing.
- [ ] Shims are exposed atomically and do not use system PHP implicitly.
```

### Feature: Daemon And Supervisor With Mailpit

**Labels:** `feature`, `priority-critical`, `value-high`, `resource`, `control-plane`

```markdown
## Feature Description

Add the daemon reconcile loop, resource-agnostic supervisor, and Mailpit as the
first supervised runnable resource.

## Parent Epic

Epic: Runtime, Daemon, And Resources

## Stories And Enablers

- [ ] Enabler: Add Daemon Reconcile Loop
- [ ] Enabler: Add Resource-Agnostic Supervisor
- [ ] User Story: Manage Mailpit As First Runnable Resource
- [ ] User Story: Record Runnable Resource Observed Status
- [ ] Test: Daemon And Supervisor Process Lifecycle

## Dependencies

Blocked by:

- Epic 2 store and signaling seams

Blocks:

- Stateful database resources
- Cache, mail, and object storage resources
- Epic 5 status UX

## Acceptance Criteria

- [ ] Daemon can reconcile desired resource records.
- [ ] Durable state changes can wake or signal reconciliation.
- [ ] Supervisor start/stop/check APIs are resource-agnostic.
- [ ] Mailpit process definition includes ports, logs, readiness, and env values.
- [ ] Observed status records PID, port, log path, failure, and last reconcile time.
```

### Feature: Stateful Database Resources

**Labels:** `feature`, `priority-critical`, `value-high`, `resource`

```markdown
## Feature Description

Add Postgres and MySQL as explicit stateful database resources with version-line
state, process lifecycle, readiness, env values, and explicit database commands.

## Parent Epic

Epic: Runtime, Daemon, And Resources

## Stories And Enablers

- [ ] User Story: Manage Postgres Version-Line Resource
- [ ] User Story: Expose Postgres Env Values And Explicit Database Commands
- [ ] User Story: Manage MySQL Version-Line Resource
- [ ] User Story: Preserve MySQL-Specific Initialization And Process Semantics
- [ ] Test: Stateful Database Resource Behavior

## Dependencies

Blocked by:

- Feature: Daemon And Supervisor With Mailpit
- Epic 2 install planner

Blocks:

- Epic 4 Laravel env, setup, and database helper commands

## Acceptance Criteria

- [ ] Postgres has version-line desired state, data/log paths, process definition, readiness, env values, and status.
- [ ] Postgres `db:create`, `db:drop`, and `db:list` commands are explicit.
- [ ] MySQL has version-line desired state, initialization, socket/PID behavior, privileges, process definition, readiness, env values, and status.
- [ ] MySQL `db:create`, `db:drop`, and `db:list` commands are explicit.
- [ ] Shared mechanics are extracted only where both resources prove the same shape.
```

### Feature: Cache, Mail, And Object Storage Resources

**Labels:** `feature`, `priority-high`, `value-high`, `resource`

```markdown
## Feature Description

Add Redis and RustFS resource behavior and expose cache, mail, and S3 env values
for Laravel project linking.

## Parent Epic

Epic: Runtime, Daemon, And Resources

## Stories And Enablers

- [ ] User Story: Manage Redis Runnable Stateful Resource
- [ ] User Story: Manage RustFS S3 Resource
- [ ] User Story: Expose Cache, Mail, And Object Storage Env Values
- [ ] Test: Redis, Mailpit, And RustFS Resource Behavior

## Dependencies

Blocked by:

- Feature: Daemon And Supervisor With Mailpit

Blocks:

- Epic 4 Laravel mail and s3 helper commands

## Acceptance Criteria

- [ ] Redis has desired state, process flags, data/log paths, readiness, env values, and status.
- [ ] RustFS has desired state, credential model, API and console ports, data/log paths, readiness, env values, and status.
- [ ] Secret values are not printed in status or logs.
- [ ] Mailpit remains explicit mail capture behavior, not a generic HTTP-service abstraction.
```

## Story And Enabler Issues

### Enabler: Add PHP Runtime Desired-State Controller

**Labels:** `enabler`, `priority-critical`, `runtime`, `control-plane`

```markdown
## Enabler Description

Add the PHP runtime desired-state model and controller.

## Acceptance Criteria

- [ ] PHP runtime desired state includes version identity.
- [ ] Controller uses canonical runtime paths.
- [ ] Controller integrates with the install planner.
- [ ] Observed status distinguishes missing, pending, ready, blocked, and failed states.
```

### User Story: Request PHP Runtime Install Desired State

**Labels:** `user-story`, `priority-critical`, `runtime`, `control-plane`

```markdown
## Story Statement

As a Laravel developer, I want to request a pv-managed PHP runtime so that my
project does not depend on system PHP.

## Acceptance Criteria

- [ ] CLI request writes desired state for the requested PHP version.
- [ ] Command does not perform hidden unrelated setup.
- [ ] Human status is written to stderr.
- [ ] Errors explain invalid or unsupported versions.
```

### Enabler: Add Composer Tool Dependency On PHP Runtime

**Labels:** `enabler`, `priority-critical`, `runtime`, `control-plane`

```markdown
## Enabler Description

Model Composer as a tool that depends on a managed PHP runtime.

## Acceptance Criteria

- [ ] Composer desired state records required PHP runtime version.
- [ ] Composer controller checks runtime availability before reconcile.
- [ ] Missing runtime creates blocked observed status with next action.
- [ ] Composer behavior does not silently fall back to system PHP.
```

### User Story: Expose Runtime-Aware CLI Shims Atomically

**Labels:** `user-story`, `priority-critical`, `runtime`, `control-plane`

```markdown
## Story Statement

As a Laravel developer, I want PHP and Composer shims to target managed runtimes
so that command behavior is deterministic across projects.

## Acceptance Criteria

- [ ] PHP shim points at the managed runtime path.
- [ ] Composer shim runs through the selected managed PHP runtime.
- [ ] Shim replacement is atomic.
- [ ] Failed shim writes do not advertise ready status.
```

### Test: PHP And Composer Runtime Dependency Behavior

**Labels:** `test`, `priority-high`, `runtime`, `control-plane`

```markdown
## Test Description

Validate PHP runtime and Composer dependency behavior.

## Acceptance Criteria

- [ ] Tests cover PHP desired-state persistence.
- [ ] Tests cover Composer blocked status when PHP is missing.
- [ ] Tests cover atomic shim exposure and failure behavior.
- [ ] Tests prove system PHP is not used as an implicit fallback.
```

### Enabler: Add Daemon Reconcile Loop

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Add the daemon loop that reconciles durable desired-state records.

## Acceptance Criteria

- [ ] Daemon enumerates desired resources.
- [ ] Daemon dispatches reconcile work to controllers.
- [ ] Durable state changes can signal or wake the daemon.
- [ ] Reconcile errors become observed status instead of panics.
```

### Enabler: Add Resource-Agnostic Supervisor

**Labels:** `enabler`, `priority-critical`, `resource`, `control-plane`

```markdown
## Enabler Description

Add the supervisor process lifecycle API without resource-specific knowledge.

## Acceptance Criteria

- [ ] Supervisor accepts process definitions.
- [ ] Supervisor can start, stop, check, and report readiness.
- [ ] Restart budget behavior is deterministic.
- [ ] Supervisor package does not mention specific resources.
```

### User Story: Manage Mailpit As First Runnable Resource

**Labels:** `user-story`, `priority-critical`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want Mailpit managed by pv so that local mail capture
is available through declared project resources.

## Acceptance Criteria

- [ ] Mailpit desired state can be represented.
- [ ] Mailpit process definition includes SMTP and web ports.
- [ ] Mailpit readiness is reported.
- [ ] Mailpit env values are available to project linking.
```

### User Story: Record Runnable Resource Observed Status

**Labels:** `user-story`, `priority-critical`, `resource`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want runnable resources to record observed process status so
that failures are explainable.

## Acceptance Criteria

- [ ] Observed status includes PID when running.
- [ ] Observed status includes port and log path when available.
- [ ] Observed status includes last error and last reconcile time.
- [ ] Crashed and restart-exhausted states include next action.
```

### Test: Daemon And Supervisor Process Lifecycle

**Labels:** `test`, `priority-high`, `resource`, `control-plane`

```markdown
## Test Description

Validate daemon reconciliation and supervisor process lifecycle behavior.

## Acceptance Criteria

- [ ] Tests cover daemon wake-up after durable state changes.
- [ ] Tests cover supervisor start, stop, readiness, crash, and restart budget.
- [ ] Tests prove supervisor remains resource-agnostic.
- [ ] Tests use fake processes by default.
```

### User Story: Manage Postgres Version-Line Resource

**Labels:** `user-story`, `priority-critical`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want pv to manage a declared Postgres version line so
that database state and process behavior are predictable.

## Acceptance Criteria

- [ ] Postgres desired state includes version line.
- [ ] Postgres data and log paths use canonical layout helpers.
- [ ] Postgres process definition and readiness are implemented.
- [ ] Status covers missing install, stopped, running, blocked, and failed states.
```

### User Story: Expose Postgres Env Values And Explicit Database Commands

**Labels:** `user-story`, `priority-critical`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want Postgres env values and explicit database
commands so that setup is reviewable and scriptable.

## Acceptance Criteria

- [ ] Postgres exposes host, port, database, username, password, and URL values as applicable.
- [ ] Database create, drop, and list commands are explicit.
- [ ] Commands target the declared resource.
- [ ] Missing resource errors are clear.
```

### User Story: Manage MySQL Version-Line Resource

**Labels:** `user-story`, `priority-critical`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want pv to manage a declared MySQL version line so
that MySQL projects do not depend on external local setup.

## Acceptance Criteria

- [ ] MySQL desired state includes version line.
- [ ] MySQL data and log paths use canonical layout helpers.
- [ ] MySQL process definition and readiness are implemented.
- [ ] Status covers missing install, stopped, running, blocked, and failed states.
```

### User Story: Preserve MySQL-Specific Initialization And Process Semantics

**Labels:** `user-story`, `priority-critical`, `resource`

```markdown
## Story Statement

As a maintainer, I want MySQL-specific initialization and process behavior to
remain explicit so that database abstractions do not hide important differences.

## Acceptance Criteria

- [ ] Initialization is explicit and idempotent where possible.
- [ ] Socket and PID behavior are represented where needed.
- [ ] Privilege handling is documented in code and tests.
- [ ] Shared database mechanics do not erase MySQL-specific behavior.
```

### Test: Stateful Database Resource Behavior

**Labels:** `test`, `priority-high`, `resource`

```markdown
## Test Description

Validate Postgres and MySQL resource behavior.

## Acceptance Criteria

- [ ] Tests cover version-line desired state.
- [ ] Tests cover data/log paths and process definitions.
- [ ] Tests cover env values and explicit database commands.
- [ ] Tests cover MySQL-specific initialization, socket/PID, and privilege behavior.
```

### User Story: Manage Redis Runnable Stateful Resource

**Labels:** `user-story`, `priority-high`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want pv to manage Redis so that cache and queue
resources can be declared locally.

## Acceptance Criteria

- [ ] Redis desired state includes version line.
- [ ] Redis process flags, data path, and log path are explicit.
- [ ] Redis readiness and status are implemented.
- [ ] Redis env values are available to project linking.
```

### User Story: Manage RustFS S3 Resource

**Labels:** `user-story`, `priority-high`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want pv to manage RustFS so that S3-compatible local
object storage can be declared.

## Acceptance Criteria

- [ ] RustFS desired state includes version identity.
- [ ] RustFS credential model is explicit.
- [ ] API and console ports are represented.
- [ ] S3 env values are available without printing secret values in status.
```

### User Story: Expose Cache, Mail, And Object Storage Env Values

**Labels:** `user-story`, `priority-high`, `resource`, `laravel`

```markdown
## Story Statement

As a Laravel developer, I want cache, mail, and object storage env values
available from declared resources so that `pv link` can render reviewable
configuration later.

## Acceptance Criteria

- [ ] Redis env values are exposed by the resource controller.
- [ ] Mailpit env values are exposed by the resource controller.
- [ ] RustFS/S3 env values are exposed by the resource controller.
- [ ] No env values are inferred from `.env`.
```

### Test: Redis, Mailpit, And RustFS Resource Behavior

**Labels:** `test`, `priority-high`, `resource`

```markdown
## Test Description

Validate Redis, Mailpit, and RustFS resource behavior.

## Acceptance Criteria

- [ ] Tests cover Redis process flags, data/log paths, readiness, env values, and status.
- [ ] Tests cover Mailpit ports, readiness, env values, and explicit mail-capture behavior.
- [ ] Tests cover RustFS credentials, API/console ports, env values, redaction, and status.
- [ ] Tests prove secret-like values are not printed.
```
