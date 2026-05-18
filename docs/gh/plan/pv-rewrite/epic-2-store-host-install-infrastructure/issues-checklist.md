# Issues Checklist: Epic 2 - Store, Host, And Install Infrastructure

Create these issues after Epic 1 is published.

## Published Issues

Milestone: `pv rewrite MVP`

| Type | Issue | Title |
| --- | --- | --- |
| Epic | #128 | Epic: Store, Host, And Install Infrastructure |
| Feature | #129 | Feature: Store And Filesystem Guardrails |
| Feature | #130 | Feature: Scriptable Install Planner |
| Enabler | #131 | Enabler: Add Canonical Host Path Helpers |
| Enabler | #132 | Enabler: Add Store Schema And Migration Seam |
| Enabler | #133 | Enabler: Decide Project Contract Version Path |
| User Story | #134 | User Story: Prevent Ambiguous Storage Locations |
| Test | #135 | Test: Store Migration And Filesystem Layout |
| Enabler | #136 | Enabler: Add Install Plan Model And Dependency Graph |
| Enabler | #137 | Enabler: Add Bounded Download Scheduler |
| User Story | #138 | User Story: Execute Dependency Ordered Installs |
| User Story | #139 | User Story: Expose Shims Atomically After Install |
| User Story | #140 | User Story: Persist Desired State And Signal Reconciliation After Durable Install |
| Test | #141 | Test: Install Planner Scheduling And Failure Behavior |

Tracker hygiene performed:

- Legacy flat issues #110 and #112 remain reference-only.
- Added superseded/reference comments to #110 and #112.
- Added `ready-for-agent` to Epic 2 leaf issues #131-#141.

## Epic Issue

### Title

`Epic: Store, Host, And Install Infrastructure`

### Labels

`epic`, `priority-critical`, `value-high`, `control-plane`

### Body

```markdown
## Epic Description

Build the store, host path, migration, and install-planning infrastructure that
prevents architecture drift before more resources are added.

Legacy references: #110, #112.

## Business Value

- `~/.pv` has explicit layout rules.
- Machine-owned state has visible schema and migration seams.
- Install/update workflows share deterministic planning instead of duplicating
  command-specific glue.

## Features

- [ ] Feature: Store And Filesystem Guardrails
- [ ] Feature: Scriptable Install Planner

## Acceptance Criteria

- [ ] Canonical path helpers exist for every target layout family.
- [ ] Store has schema version and applied migration seam.
- [ ] Contract versioning is implemented or explicitly deferred with a clear path.
- [ ] Install plans support runtimes, tools, and services.
- [ ] Downloads are bounded.
- [ ] Installs are dependency ordered.
- [ ] Shim exposure is atomic.
- [ ] Failed plans do not advertise completed work.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Root verification passes.
- [ ] No expensive artifact workflows were run unless explicitly requested.
```

## Feature Issues

### Feature: Store And Filesystem Guardrails

**Labels:** `feature`, `priority-critical`, `value-high`, `control-plane`

```markdown
## Feature Description

Add the canonical host path helpers, store schema/migration seam, contract
versioning decision, and layout validation needed before more resources are
implemented.

## Parent Epic

Epic: Store, Host, And Install Infrastructure

## Stories And Enablers

- [ ] Enabler: Add Canonical Host Path Helpers
- [ ] Enabler: Add Store Schema And Migration Seam
- [ ] Enabler: Decide Project Contract Version Path
- [ ] User Story: Prevent Ambiguous Storage Locations
- [ ] Test: Store Migration And Filesystem Layout

## Dependencies

Blocked by:

- Epic 1: Rewrite Foundation

Blocks:

- Feature: Scriptable Install Planner
- Epic 3 runtime and resource work
- Epic 4 project contract and link work

## Acceptance Criteria

- [ ] Path helpers cover bin, runtimes, tools, services, data, logs, state, cache, and config.
- [ ] Store exposes schema version.
- [ ] Applied migrations can be recorded.
- [ ] Contract versioning path is explicit.
- [ ] Tests prevent ambiguous binary/data locations.
```

### Feature: Scriptable Install Planner

**Labels:** `feature`, `priority-high`, `value-high`, `control-plane`

```markdown
## Feature Description

Add shared install planning for runtimes, tools, and services: dependency graph,
bounded downloads, dependency-ordered installs, atomic shim exposure, durable
state persistence, and daemon signaling seam.

## Parent Epic

Epic: Store, Host, And Install Infrastructure

## Stories And Enablers

- [ ] Enabler: Add Install Plan Model And Dependency Graph
- [ ] Enabler: Add Bounded Download Scheduler
- [ ] User Story: Execute Dependency Ordered Installs
- [ ] User Story: Expose Shims Atomically After Install
- [ ] User Story: Persist Desired State And Signal Reconciliation After Durable Install
- [ ] Test: Install Planner Scheduling And Failure Behavior

## Dependencies

Blocked by:

- Feature: Store And Filesystem Guardrails

Blocks:

- Epic 3 runtime, tool, and service install flows

## Acceptance Criteria

- [ ] Plans can include runtimes, tools, and services.
- [ ] Invalid plans fail before work starts.
- [ ] Downloads run with bounded parallelism.
- [ ] Installs run in dependency order.
- [ ] Shims are exposed atomically.
- [ ] Failed plans leave clear results and no advertised completed work.
```

## Story And Enabler Issues

### Enabler: Add Canonical Host Path Helpers

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Add host path helpers for the canonical `~/.pv` layout.

## Acceptance Criteria

- [ ] Helpers exist for bin, runtimes, tools, services, data, logs, state, cache, and config.
- [ ] Helpers validate resource names and version path segments.
- [ ] Active rewrite path construction uses the helpers.
- [ ] Tests isolate `HOME`.
```

### Enabler: Add Store Schema And Migration Seam

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Add schema versioning and applied migration records to the machine-owned store.

## Acceptance Criteria

- [ ] Store exposes a schema version.
- [ ] Applied migrations can be recorded.
- [ ] Migration runner executes pending migrations in order.
- [ ] Migration failures return clear errors and do not silently reinterpret state.
```

### Enabler: Decide Project Contract Version Path

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Record the `pv.yml` contract versioning decision: new rewrite contracts use
top-level `version: 1`, and Epic 4 issue #171 implements parser validation.

## Acceptance Criteria

- [ ] Decision is documented as top-level `version: 1`.
- [ ] Epic 4 issue #171 is named as the parser/validation owner.
- [ ] Epic 2 does not add full contract parser code.
```

### User Story: Prevent Ambiguous Storage Locations

**Labels:** `user-story`, `priority-critical`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want pv storage paths to be validated so that binaries, data,
logs, state, and cache cannot drift into ambiguous locations.

## Acceptance Criteria

- [ ] Real binaries cannot be represented as top-level `~/.pv/bin` files.
- [ ] Stateful data paths live under `data`.
- [ ] Logs live under `logs`.
- [ ] Services cannot invent top-level path families.
- [ ] Unsafe path segments are rejected.
```

### Enabler: Add Install Plan Model And Dependency Graph

**Labels:** `enabler`, `priority-high`, `control-plane`

```markdown
## Enabler Description

Create the install plan item model and dependency graph for runtimes, tools, and
services.

## Acceptance Criteria

- [ ] Plan items include kind, name, and version identity.
- [ ] Plans reject duplicate identities.
- [ ] Plans reject missing dependencies.
- [ ] Topological order is deterministic.
```

### Enabler: Add Bounded Download Scheduler

**Labels:** `enabler`, `priority-high`, `control-plane`

```markdown
## Enabler Description

Add bounded parallel download scheduling behind a downloader adapter.

## Acceptance Criteria

- [ ] Parallelism never exceeds configured bound.
- [ ] Context cancellation is honored.
- [ ] Per-item failures are reported clearly.
- [ ] Tests use fake downloader adapters.
```

### User Story: Execute Dependency Ordered Installs

**Labels:** `user-story`, `priority-high`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want install plans to execute in dependency order so that
tools and services do not install before prerequisites are ready.

## Acceptance Criteria

- [ ] Installer adapter is used.
- [ ] Items execute in dependency order.
- [ ] Failed prerequisites skip dependent work.
- [ ] Results distinguish ready, skipped, and failed items.
```

### User Story: Expose Shims Atomically After Install

**Labels:** `user-story`, `priority-high`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want shims exposed atomically after install so users never
run partial or stale tool entrypoints.

## Acceptance Criteria

- [ ] Shim writer writes temp file in target directory.
- [ ] Shim permissions are set before exposure.
- [ ] Rename into place is atomic.
- [ ] Temp files are cleaned on failure.
```

### User Story: Persist Desired State And Signal Reconciliation After Durable Install

**Labels:** `user-story`, `priority-high`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want install completion to persist desired state and signal
reconciliation only after durable work succeeds.

## Acceptance Criteria

- [ ] Desired state is persisted after successful durable install work.
- [ ] Daemon signal seam is called only after persistence succeeds.
- [ ] Failed plans do not signal reconciliation.
- [ ] Failed plans do not advertise completed work.
```

## Test Issues

### Test: Store Migration And Filesystem Layout

**Labels:** `test`, `priority-high`, `control-plane`

```markdown
## Test Scope

Validate Feature: Store And Filesystem Guardrails.

## Test Cases

- [ ] Canonical path helpers produce expected paths.
- [ ] Tests isolate `HOME`.
- [ ] Unsafe path segments are rejected.
- [ ] Schema version is stored.
- [ ] Applied migrations are recorded.
- [ ] Migration failure returns clear error.
- [ ] Ambiguous binary/data paths are prevented.
```

### Test: Install Planner Scheduling And Failure Behavior

**Labels:** `test`, `priority-high`, `control-plane`

```markdown
## Test Scope

Validate Feature: Scriptable Install Planner.

## Test Cases

- [ ] Plans validate duplicate identities.
- [ ] Plans validate missing dependencies.
- [ ] Dependency ordering is deterministic.
- [ ] Download parallelism is bounded.
- [ ] Context cancellation stops scheduling.
- [ ] Failed prerequisites skip dependent installs.
- [ ] Atomic shim writer cleans temporary files.
- [ ] Successful durable plans persist state then signal.
- [ ] Failed plans do not signal or advertise completed work.
```
