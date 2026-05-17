# Issues Checklist: Epic 4 - Laravel Project Experience

Create these issues after Epic 3 is published.

## Published Issues

Milestone: `pv rewrite MVP`

| Type | Issue | Title |
| --- | --- | --- |
| Epic | #166 | Epic: Laravel Project Experience |
| Feature | #167 | Feature: Project Contract And Init |
| Feature | #168 | Feature: Link, Env, And Setup |
| Feature | #169 | Feature: Gateway And pv open |
| Feature | #170 | Feature: Laravel Helper Commands |
| Enabler | #171 | Enabler: Add Project Contract Schema And Parser |
| Enabler | #172 | Enabler: Add Laravel Detection And Contract Generator |
| User Story | #173 | User Story: Generate Reviewable pv.yml With pv init |
| User Story | #174 | User Story: Refuse Contract Overwrite Unless Forced |
| Test | #175 | Test: Project Contract And Init Behavior |
| Enabler | #176 | Enabler: Add Project Registry Desired-State Model |
| Enabler | #177 | Enabler: Add Managed Env Merge Writer |
| Enabler | #178 | Enabler: Add Setup Runner With Pinned Runtime |
| User Story | #179 | User Story: Link Laravel Project From pv.yml |
| User Story | #180 | User Story: Fail Fast On Missing Services Or Setup Errors |
| Test | #181 | Test: Link Env And Setup Behavior |
| Enabler | #182 | Enabler: Add Gateway Desired And Observed State |
| Enabler | #183 | Enabler: Add Deterministic FrankenPHP/Caddy Route Rendering |
| Enabler | #184 | Enabler: Add TLS And DNS Host Adapters |
| User Story | #185 | User Story: Serve Linked Laravel App At HTTPS Test Host |
| User Story | #186 | User Story: Open Linked Laravel App With pv open |
| Test | #187 | Test: Gateway And pv open Behavior |
| User Story | #188 | User Story: Run Artisan Through Pinned PHP Runtime |
| User Story | #189 | User Story: Route Database Helper To Declared Database Resource |
| User Story | #190 | User Story: Route Mail Helper To Declared Mailpit Resource |
| User Story | #191 | User Story: Route S3 Helper To Declared RustFS Resource |
| Test | #192 | Test: Laravel Helper Command Routing |

Tracker hygiene performed:

- Legacy flat issues #106-#108 and #111 remain reference-only.
- Added superseded/reference comments to #106-#108 and #111.
- Added `ready-for-agent` to Epic 4 leaf issues #171-#192.
- Updated Epic 4 container issue bodies with child issue links.

## Epic Issue

### Title

`Epic: Laravel Project Experience`

### Labels

`epic`, `priority-critical`, `value-high`, `laravel`, `gateway`

### Body

```markdown
## Epic Description

Build the Laravel-first project workflow: `pv init`, `pv link`, managed env
writes, setup commands, HTTPS `.test` routing, `pv open`, and Laravel helper
commands.

Legacy references: #106, #107, #108, #111.

## Business Value

- Laravel developers get the daily Herd replacement workflow.
- Project configuration is explicit and reviewable through `pv.yml`.
- Local env and setup behavior is controlled by declarations, not hidden
  inference.

## Features

- [ ] Feature: Project Contract And Init
- [ ] Feature: Link, Env, And Setup
- [ ] Feature: Gateway And pv open
- [ ] Feature: Laravel Helper Commands

## Acceptance Criteria

- [ ] `pv init` detects Laravel projects and generates a reviewable `pv.yml`.
- [ ] Existing contracts are not overwritten unless forced.
- [ ] `pv link` records durable desired project state.
- [ ] Env writes are labeled, declared-only, and backed up.
- [ ] Setup commands run through managed PHP and fail fast.
- [ ] Linked apps serve at HTTPS `.test` hosts.
- [ ] `pv open` opens the linked app through an adapter.
- [ ] Helper commands route through current project and declared resources.
- [ ] No services or env values are inferred from `.env`.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Root verification passes.
- [ ] Gateway OS mutation is adapter-tested or documented as manual QA.
```

## Feature Issues

### Feature: Project Contract And Init

**Labels:** `feature`, `priority-critical`, `value-high`, `laravel`

```markdown
## Feature Description

Add versioned `pv.yml` parsing, Laravel detection, deterministic contract
generation, `pv init`, and overwrite protection.

## Parent Epic

Epic: Laravel Project Experience

## Stories And Enablers

- [ ] Enabler: Add Project Contract Schema And Parser
- [ ] Enabler: Add Laravel Detection And Contract Generator
- [ ] User Story: Generate Reviewable pv.yml With pv init
- [ ] User Story: Refuse Contract Overwrite Unless Forced
- [ ] Test: Project Contract And Init Behavior

## Dependencies

Blocked by:

- Epic 2 contract versioning decision

Blocks:

- Feature: Link, Env, And Setup

## Acceptance Criteria

- [ ] Contract parser validates version, PHP, services, aliases, and setup commands.
- [ ] Laravel detection uses explicit project markers.
- [ ] Generated `pv.yml` is deterministic.
- [ ] Existing contract overwrite requires force.
```

### Feature: Link, Env, And Setup

**Labels:** `feature`, `priority-critical`, `value-high`, `laravel`, `control-plane`

```markdown
## Feature Description

Resolve `pv.yml` during `pv link`, record durable project desired state, write
declared env values safely, and run declared setup commands through managed PHP.

## Parent Epic

Epic: Laravel Project Experience

## Stories And Enablers

- [ ] Enabler: Add Project Registry Desired-State Model
- [ ] Enabler: Add Managed Env Merge Writer
- [ ] Enabler: Add Setup Runner With Pinned Runtime
- [ ] User Story: Link Laravel Project From pv.yml
- [ ] User Story: Fail Fast On Missing Services Or Setup Errors
- [ ] Test: Link Env And Setup Behavior

## Dependencies

Blocked by:

- Feature: Project Contract And Init
- Epic 3 PHP runtime and resource env providers

Blocks:

- Gateway and helper command work

## Acceptance Criteria

- [ ] `pv link` records desired project state durably.
- [ ] `.env` writes are labeled, declared-only, and backed up.
- [ ] Setup commands run in project directory through managed PHP.
- [ ] Missing resources and setup failures return actionable errors.
```

### Feature: Gateway And pv open

**Labels:** `feature`, `priority-critical`, `value-high`, `gateway`, `laravel`

```markdown
## Feature Description

Serve linked Laravel apps at HTTPS `.test` hosts through gateway desired state,
deterministic route rendering, DNS/TLS adapters, and `pv open`.

## Parent Epic

Epic: Laravel Project Experience

## Stories And Enablers

- [ ] Enabler: Add Gateway Desired And Observed State
- [ ] Enabler: Add Deterministic FrankenPHP/Caddy Route Rendering
- [ ] Enabler: Add TLS And DNS Host Adapters
- [ ] User Story: Serve Linked Laravel App At HTTPS Test Host
- [ ] User Story: Open Linked Laravel App With pv open
- [ ] Test: Gateway And pv open Behavior

## Dependencies

Blocked by:

- Feature: Link, Env, And Setup
- Epic 3 daemon and supervisor

Blocks:

- Epic 5 status and release QA

## Acceptance Criteria

- [ ] Gateway desired and observed state includes linked project hosts.
- [ ] Route rendering is deterministic.
- [ ] TLS SANs cover primary host and aliases.
- [ ] DNS and browser open behavior is adapter-driven.
- [ ] `pv open` resolves the current linked project.
```

### Feature: Laravel Helper Commands

**Labels:** `feature`, `priority-high`, `value-high`, `laravel`

```markdown
## Feature Description

Add Laravel helper commands for Artisan, database, mail, and S3 workflows that
route through managed PHP and declared resources.

## Parent Epic

Epic: Laravel Project Experience

## Stories And Enablers

- [ ] User Story: Run Artisan Through Pinned PHP Runtime
- [ ] User Story: Route Database Helper To Declared Database Resource
- [ ] User Story: Route Mail Helper To Declared Mailpit Resource
- [ ] User Story: Route S3 Helper To Declared RustFS Resource
- [ ] Test: Laravel Helper Command Routing

## Dependencies

Blocked by:

- Feature: Link, Env, And Setup
- Epic 3 resource controllers

## Acceptance Criteria

- [ ] Helper commands resolve the current linked project.
- [ ] Artisan runs through managed PHP.
- [ ] Database helper targets declared Postgres or MySQL resource.
- [ ] Mail helper targets declared Mailpit resource.
- [ ] S3 helper targets declared RustFS resource.
```

## Story And Enabler Issues

### Enabler: Add Project Contract Schema And Parser

**Labels:** `enabler`, `priority-critical`, `laravel`, `control-plane`

```markdown
## Enabler Description

Add the versioned project contract schema and parser for `pv.yml`, requiring
top-level `version: 1` for new rewrite contracts.

## Acceptance Criteria

- [ ] Parser reads top-level `version: 1`.
- [ ] Parser validates PHP version, service declarations, aliases, and setup commands.
- [ ] Unknown or unsupported fields return clear errors.
- [ ] Tests cover valid and invalid contracts.
```

### Enabler: Add Laravel Detection And Contract Generator

**Labels:** `enabler`, `priority-critical`, `laravel`

```markdown
## Enabler Description

Add Laravel project detection and deterministic contract generation.

## Acceptance Criteria

- [ ] Laravel detection uses explicit project markers.
- [ ] Generator produces stable YAML ordering.
- [ ] Defaults are Laravel-first but reviewable.
- [ ] Service choices are not inferred from `.env`.
```

### User Story: Generate Reviewable pv.yml With pv init

**Labels:** `user-story`, `priority-critical`, `laravel`

```markdown
## Story Statement

As a Laravel developer, I want `pv init` to generate a reviewable `pv.yml` so
that project infrastructure is explicit before linking.

## Acceptance Criteria

- [ ] `pv init` writes a valid `pv.yml`.
- [ ] Generated contract includes version and PHP declaration.
- [ ] Generated contract is deterministic.
- [ ] Command does not install resources or mutate `.env`.
```

### User Story: Refuse Contract Overwrite Unless Forced

**Labels:** `user-story`, `priority-critical`, `laravel`

```markdown
## Story Statement

As a Laravel developer, I want `pv init` to preserve existing contracts unless I
force an overwrite so that local project configuration is not lost.

## Acceptance Criteria

- [ ] Existing `pv.yml` is not overwritten by default.
- [ ] Forced overwrite is explicit.
- [ ] Error output explains the next action.
- [ ] Existing file permissions are handled predictably.
```

### Test: Project Contract And Init Behavior

**Labels:** `test`, `priority-high`, `laravel`

```markdown
## Test Description

Validate contract parsing, Laravel detection, deterministic generation, and
`pv init` overwrite behavior.

## Acceptance Criteria

- [ ] Tests cover valid and invalid `pv.yml` contracts.
- [ ] Tests cover Laravel detection markers.
- [ ] Tests cover deterministic output.
- [ ] Tests cover overwrite refusal and forced overwrite.
```

### Enabler: Add Project Registry Desired-State Model

**Labels:** `enabler`, `priority-critical`, `laravel`, `control-plane`

```markdown
## Enabler Description

Add the durable project registry or project desired-state model used by `pv link`.

## Acceptance Criteria

- [ ] Project desired state records project path, host, aliases, contract version, PHP, services, and setup commands.
- [ ] Project identity is deterministic.
- [ ] Store writes happen before daemon signal.
- [ ] Tests isolate `HOME`.
```

### Enabler: Add Managed Env Merge Writer

**Labels:** `enabler`, `priority-critical`, `laravel`

```markdown
## Enabler Description

Add an env parser and merge writer that updates only pv-managed declarations.

## Acceptance Criteria

- [ ] Writer preserves user-authored env lines.
- [ ] Writer labels pv-managed blocks or keys.
- [ ] Writer backs up `.env` before mutation.
- [ ] Removed declarations update only pv-managed entries.
```

### Enabler: Add Setup Runner With Pinned Runtime

**Labels:** `enabler`, `priority-critical`, `laravel`, `runtime`

```markdown
## Enabler Description

Add setup command execution with project working directory, managed PATH, pinned
PHP runtime, and streamed output.

## Acceptance Criteria

- [ ] Setup commands run in project directory.
- [ ] PATH resolves managed PHP before system PHP.
- [ ] stdout and stderr stream predictably.
- [ ] Runner stops on first failed setup command.
```

### User Story: Link Laravel Project From pv.yml

**Labels:** `user-story`, `priority-critical`, `laravel`, `control-plane`

```markdown
## Story Statement

As a Laravel developer, I want `pv link` to apply my `pv.yml` contract so that
the project becomes managed by pv.

## Acceptance Criteria

- [ ] `pv link` validates `pv.yml`.
- [ ] `pv link` records durable project desired state.
- [ ] `pv link` writes only declared env values.
- [ ] `pv link` signals daemon after durable state changes.
```

### User Story: Fail Fast On Missing Services Or Setup Errors

**Labels:** `user-story`, `priority-critical`, `laravel`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want link/setup failures to stop immediately with clear
next actions so that partial setup is easy to recover from.

## Acceptance Criteria

- [ ] Missing declared services produce actionable errors.
- [ ] Missing runtime/tool installs produce actionable errors.
- [ ] Setup stops on first failed command.
- [ ] Failure status is persisted for link, setup, gateway, and helper failures that affect project observed state.
```

### Test: Link Env And Setup Behavior

**Labels:** `test`, `priority-high`, `laravel`, `control-plane`

```markdown
## Test Description

Validate `pv link`, env merge behavior, setup execution, and failure handling.

## Acceptance Criteria

- [ ] Tests prove `pv link` does not infer services from `.env`.
- [ ] Tests cover managed env writes, updates, removal, and backups.
- [ ] Tests cover setup PATH pinning and fail-fast behavior.
- [ ] Tests cover daemon signal ordering after durable state writes.
```

### Enabler: Add Gateway Desired And Observed State

**Labels:** `enabler`, `priority-critical`, `gateway`, `control-plane`

```markdown
## Enabler Description

Add gateway desired and observed state for linked Laravel projects.

## Acceptance Criteria

- [ ] Desired state includes primary host, aliases, project path, and runtime reference.
- [ ] Observed state records route status and failure information.
- [ ] Gateway status can be aggregated with project status.
- [ ] Tests cover missing project and stale route states.
```

### Enabler: Add Deterministic FrankenPHP/Caddy Route Rendering

**Labels:** `enabler`, `priority-critical`, `gateway`

```markdown
## Enabler Description

Render deterministic FrankenPHP/Caddy gateway routes for linked projects.

## Acceptance Criteria

- [ ] Rendering is stable across runs.
- [ ] Primary host and aliases are included.
- [ ] Route output is diffable.
- [ ] FrankenPHP is treated as gateway infrastructure.
```

### Enabler: Add TLS And DNS Host Adapters

**Labels:** `enabler`, `priority-critical`, `gateway`, `control-plane`

```markdown
## Enabler Description

Add host adapters for TLS certificate material, SAN behavior, DNS writes, and
testable OS integration boundaries.

## Acceptance Criteria

- [ ] TLS adapter models certificate material and SANs.
- [ ] DNS adapter avoids direct OS mutation in unit tests.
- [ ] Adapter errors are actionable.
- [ ] Privileged operations are isolated behind host primitives.
```

### User Story: Serve Linked Laravel App At HTTPS Test Host

**Labels:** `user-story`, `priority-critical`, `gateway`, `laravel`

```markdown
## Story Statement

As a Laravel developer, I want linked apps served at HTTPS `.test` hosts so that
local development feels native.

## Acceptance Criteria

- [ ] Linked project has primary `.test` host.
- [ ] Aliases are supported in route and TLS behavior.
- [ ] Gateway process definition runs through supervisor.
- [ ] Status explains missing DNS, TLS, route, or process failures.
```

### User Story: Open Linked Laravel App With pv open

**Labels:** `user-story`, `priority-critical`, `gateway`, `laravel`

```markdown
## Story Statement

As a Laravel developer, I want `pv open` to open the current linked app so that I
can jump to the browser quickly.

## Acceptance Criteria

- [ ] `pv open` resolves the current linked project.
- [ ] `pv open` uses browser-open adapter.
- [ ] Missing link or missing gateway state returns actionable errors.
- [ ] Command remains scriptable.
```

### Test: Gateway And pv open Behavior

**Labels:** `test`, `priority-high`, `gateway`, `laravel`

```markdown
## Test Description

Validate gateway route rendering, DNS/TLS adapters, supervisor integration, and
`pv open`.

## Acceptance Criteria

- [ ] Tests cover deterministic route rendering.
- [ ] Tests cover primary host and aliases.
- [ ] Tests cover TLS SAN and DNS adapter errors.
- [ ] Tests cover `pv open` current-project resolution and browser adapter.
```

### User Story: Run Artisan Through Pinned PHP Runtime

**Labels:** `user-story`, `priority-high`, `laravel`, `runtime`

```markdown
## Story Statement

As a Laravel developer, I want `pv artisan` to run through the project's managed
PHP runtime so that command behavior matches the linked project.

## Acceptance Criteria

- [ ] `pv artisan` resolves current project.
- [ ] Command uses declared managed PHP runtime.
- [ ] Arguments pass through to Artisan.
- [ ] Missing runtime errors include next action.
```

### User Story: Route Database Helper To Declared Database Resource

**Labels:** `user-story`, `priority-high`, `laravel`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want `pv db` to route to the declared database
resource so that database actions are explicit.

## Acceptance Criteria

- [ ] Helper resolves Postgres or MySQL from project contract.
- [ ] Missing database declaration returns clear error.
- [ ] Helper uses declared resource connection data.
- [ ] Commands remain scriptable.
```

### User Story: Route Mail Helper To Declared Mailpit Resource

**Labels:** `user-story`, `priority-high`, `laravel`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want `pv mail` to route to the declared Mailpit
resource so that mail capture is easy to inspect.

## Acceptance Criteria

- [ ] Helper resolves declared Mailpit resource.
- [ ] Missing Mailpit declaration returns clear error.
- [ ] Helper uses Mailpit web route or status data.
- [ ] Command remains scriptable.
```

### User Story: Route S3 Helper To Declared RustFS Resource

**Labels:** `user-story`, `priority-high`, `laravel`, `resource`

```markdown
## Story Statement

As a Laravel developer, I want `pv s3` to route to the declared RustFS resource
so that local object storage actions are explicit.

## Acceptance Criteria

- [ ] Helper resolves declared RustFS resource.
- [ ] Missing RustFS declaration returns clear error.
- [ ] Helper does not print secret values.
- [ ] Command remains scriptable.
```

### Test: Laravel Helper Command Routing

**Labels:** `test`, `priority-high`, `laravel`

```markdown
## Test Description

Validate current-project resolution and helper routing for Artisan, database,
mail, and S3 commands.

## Acceptance Criteria

- [ ] Tests cover `pv artisan` managed PHP execution.
- [ ] Tests cover `pv db` declared database routing.
- [ ] Tests cover `pv mail` declared Mailpit routing.
- [ ] Tests cover `pv s3` declared RustFS routing and secret redaction.
```
