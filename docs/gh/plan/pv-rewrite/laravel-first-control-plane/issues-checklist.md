# Issue Creation Checklist: Laravel-First Local Control Plane

Use this file to create the new GitHub issue hierarchy. The old issues #96-#113
and PRs #114-#115 are reference material only.

## Pre-Creation Checklist

- [ ] Confirm old flat issues are marked or commented as superseded by this plan.
- [ ] Create labels: `epic`, `feature`, `user-story`, `enabler`, `test`,
  `priority-critical`, `priority-high`, `priority-medium`, `value-high`,
  `value-medium`, `control-plane`, `laravel`, `runtime`, `gateway`,
  `resource`, `quality`.
- [ ] Create or select milestone: `pv rewrite MVP`.
- [ ] Create the project board fields listed in `project-plan.md`.
- [ ] Create issues in dependency order: epics, features, stories/enablers,
  tests.

## Epic Issues

### Epic: Rewrite Foundation

**Labels:** `epic`, `priority-critical`, `value-high`, `control-plane`

**Body:**

```markdown
## Epic Description

Establish the active rewrite workspace and prove the first control-plane
vertical slice.

## Business Value

- Primary goal: create a clean root module for the rewrite.
- Success metric: commands can record desired state and a controller can
  reconcile observed status.
- User impact: future work starts from a maintainable architecture instead of
  the prototype.

## Features

- [ ] Feature: Prototype Isolation And Root Scaffold
- [ ] Feature: First Desired-State Resource Tracer

## Acceptance Criteria

- [ ] Prototype is isolated under `legacy/prototype`.
- [ ] Root module builds independently.
- [ ] First tracer proves command -> desired state -> controller -> observed
  status.
- [ ] Root/prototype working rules are documented.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Root verification passes.
- [ ] Prototype verification passes if prototype files changed.
```

### Epic: Store, Host, And Install Infrastructure

**Labels:** `epic`, `priority-critical`, `value-high`, `control-plane`

**Body:**

```markdown
## Epic Description

Build the store, path, migration, host, and installer foundations that prevent
resource implementation from drifting.

## Features

- [ ] Feature: Store And Filesystem Guardrails
- [ ] Feature: Scriptable Install Planner

## Acceptance Criteria

- [ ] Canonical `~/.pv` layout helpers exist.
- [ ] Store has schema and migration seams.
- [ ] Install planner supports dependency-ordered installs and bounded
  downloads.
- [ ] Atomic shim exposure is shared instead of duplicated.
```

### Epic: Runtime, Daemon, And Resources

**Labels:** `epic`, `priority-critical`, `value-high`, `resource`

**Body:**

```markdown
## Epic Description

Implement managed runtimes, tools, daemon reconciliation, supervisor behavior,
and backing resources.

## Features

- [ ] Feature: PHP Runtime And Composer Tooling
- [ ] Feature: Daemon And Supervisor With Mailpit
- [ ] Feature: Stateful Database Resources
- [ ] Feature: Cache, Mail, And Object Storage Resources

## Acceptance Criteria

- [ ] PHP and Composer reconcile through desired state.
- [ ] Supervisor remains resource-agnostic.
- [ ] Mailpit, Postgres, MySQL, Redis, and RustFS fit the control-plane model.
- [ ] Resource-specific behavior stays explicit.
```

### Epic: Laravel Project Experience

**Labels:** `epic`, `priority-critical`, `value-high`, `laravel`

**Body:**

```markdown
## Epic Description

Deliver the Laravel-first product path: `pv init`, `pv link`, HTTPS `.test`
serving, `pv open`, and project-aware helper commands.

## Features

- [ ] Feature: Project Contract And Init
- [ ] Feature: Link, Env, And Setup
- [ ] Feature: Gateway And pv open
- [ ] Feature: Laravel Helper Commands

## Acceptance Criteria

- [ ] Laravel projects can generate and commit explicit `pv.yml`.
- [ ] Link records desired state and avoids hidden inference.
- [ ] Linked apps serve at HTTPS `.test` hosts.
- [ ] Helper commands use the project contract and declared resources.
```

### Epic: Status, Quality, And Scope Control

**Labels:** `epic`, `priority-high`, `value-high`, `quality`

**Body:**

```markdown
## Epic Description

Make the rewrite understandable, testable, and scoped. Status must explain what
was requested, what happened, what failed, where logs are, and what to do next.

## Features

- [ ] Feature: Desired And Observed Status UX
- [ ] Feature: Post-MVP Backlog

## Acceptance Criteria

- [ ] Status distinguishes desired and observed state.
- [ ] Failures include logs and next actions.
- [ ] Post-MVP scope is tracked explicitly and kept out of MVP.
```

## Feature Issues

Create these after their parent epic issues exist.

| Feature | Labels | Estimate | Blocked by |
| --- | --- | --- | --- |
| Prototype Isolation And Root Scaffold | `feature`, `priority-critical`, `control-plane` | 5 | none |
| First Desired-State Resource Tracer | `feature`, `priority-critical`, `control-plane` | 5 | Prototype Isolation And Root Scaffold |
| Store And Filesystem Guardrails | `feature`, `priority-critical`, `control-plane` | 8 | First Desired-State Resource Tracer |
| Scriptable Install Planner | `feature`, `priority-high`, `control-plane` | 8 | Store And Filesystem Guardrails |
| PHP Runtime And Composer Tooling | `feature`, `priority-critical`, `runtime` | 5 | First Desired-State Resource Tracer |
| Daemon And Supervisor With Mailpit | `feature`, `priority-critical`, `resource` | 8 | Store And Filesystem Guardrails |
| Stateful Database Resources | `feature`, `priority-critical`, `resource` | 13 | Daemon And Supervisor With Mailpit |
| Cache, Mail, And Object Storage Resources | `feature`, `priority-high`, `resource` | 13 | Daemon And Supervisor With Mailpit |
| Project Contract And Init | `feature`, `priority-critical`, `laravel` | 8 | PHP Runtime And Composer Tooling, Stateful Database Resources |
| Link, Env, And Setup | `feature`, `priority-critical`, `laravel` | 13 | Project Contract And Init |
| Gateway And pv open | `feature`, `priority-critical`, `gateway` | 13 | Link, Env, And Setup |
| Laravel Helper Commands | `feature`, `priority-high`, `laravel` | 8 | Link, Env, And Setup, Gateway And pv open |
| Desired And Observed Status UX | `feature`, `priority-critical`, `quality` | 8 | Daemon And Supervisor With Mailpit, Gateway And pv open |
| Post-MVP Backlog | `feature`, `priority-high`, `quality` | 3 | none |

### Feature Issue Body Template

```markdown
## Feature Description

{Feature summary from project-plan.md}

## Parent Epic

{Epic issue link}

## User Stories And Enablers

- [ ] {Story or enabler title}
- [ ] {Story or enabler title}

## Dependencies

Blocked by:

- {Blocking issue or "none"}

Blocks:

- {Blocked issue or "none"}

## Acceptance Criteria

- [ ] {Feature-level acceptance criterion}
- [ ] {Feature-level acceptance criterion}
- [ ] Required tests from `test-issues-checklist.md` are linked.

## Definition Of Done

- [ ] Stories/enablers are complete.
- [ ] Linked test issue is complete.
- [ ] Root verification passes for Go changes.
- [ ] User-facing docs or status output are updated when relevant.
- [ ] Deferred scope is added to the post-MVP backlog.

## Legacy Reference

Old reference issue or PR: {#number or "none"}
```

## Story And Enabler Issues

Create these under the relevant feature issues.

| Type | Title | Estimate | Acceptance criteria |
| --- | --- | --- | --- |
| Enabler | Move prototype into a reference module | 3 | Prototype builds from `legacy/prototype`; root no longer contains old command tree. |
| Enabler | Scaffold minimal root CLI | 2 | Help/version/unknown command behavior works; stdout/stderr discipline is tested. |
| Enabler | Add desired and observed store seam | 3 | Desired and observed state persist separately; corrupted state errors clearly. |
| Story | Request installable resource desired state | 2 | CLI validates version and writes desired state without installing directly. |
| Story | Reconcile first installable resource | 3 | Controller writes ready/failed observed status with deterministic tests. |
| Enabler | Add canonical path helpers | 3 | Helpers cover bin, runtimes, tools, services, data, logs, state, cache, config. |
| Enabler | Add store schema and migration seam | 5 | Schema version and applied migrations are represented and tested. |
| Enabler | Build install planner core | 5 | Plans resolve dependencies and schedule bounded downloads. |
| Story | Install PHP runtime through desired state | 3 | PHP controller records ready/failed observed status. |
| Story | Install Composer through pinned PHP runtime | 3 | Composer blocks when PHP runtime is missing and writes an atomic shim when ready. |
| Enabler | Add daemon reconcile loop | 5 | Daemon loads desired state, runs controllers, writes status, handles signal. |
| Enabler | Add resource-agnostic supervisor | 5 | Start/stop/probe/restart/log behavior has no resource-specific names. |
| Story | Manage Mailpit as first runnable resource | 3 | Mailpit process definition, ports, logs, and status reconcile through daemon. |
| Story | Manage Postgres version-line resource | 5 | Postgres install detection, process definition, env values, and DB commands work. |
| Story | Manage MySQL version-line resource | 5 | MySQL-specific init, process, privileges, env values, and DB commands work. |
| Story | Manage Redis runnable stateful resource | 3 | Redis process flags, persistence, env values, and status work. |
| Story | Manage RustFS S3 resource | 5 | RustFS credentials, routes, ports, env values, and status work without leaking secrets. |
| Story | Generate Laravel project contract | 5 | `pv init` detects Laravel, writes defaults, and refuses overwrite unless forced. |
| Story | Link project from explicit contract | 8 | Link writes desired project state and avoids `.env` service inference. |
| Story | Render managed env values | 5 | Only declared keys are updated, labels are written, removed declarations stop updates. |
| Story | Run declared setup commands | 5 | Commands run from root, fail fast, stream output, and use pinned PHP on `PATH`. |
| Story | Serve linked app through HTTPS `.test` gateway | 8 | Routes, aliases, TLS material, and gateway process definitions are tested. |
| Story | Open linked project | 2 | `pv open` resolves primary URL through project state and uses browser adapter. |
| Story | Run project-aware Laravel helpers | 5 | Artisan/db/mail/S3 helpers use contract and declared resources. |
| Story | Show desired and observed status | 5 | Status covers healthy, stopped, missing install, blocked, crashed, partial states. |
| Enabler | Track post-MVP backlog | 2 | Every out-of-scope PRD item has a deferral reason and reconsideration trigger. |

### Story Issue Body Template

```markdown
## Story Statement

As a {user type}, I want {goal}, so that {benefit}.

## Parent Feature

{Feature issue link}

## Acceptance Criteria

- [ ] {Specific, testable criterion}
- [ ] {Specific, testable criterion}
- [ ] Linked test issue covers the story.

## Implementation Notes

- {Module or package likely affected}
- {Important architectural constraint}

## Dependencies

Blocked by:

- {Blocking issue or "none"}

## Definition Of Done

- [ ] Acceptance criteria met.
- [ ] Tests added or updated.
- [ ] Root verification passes for Go changes.
- [ ] PR body lists exact test commands.
```

### Enabler Issue Body Template

```markdown
## Enabler Description

{Technical capability needed to support one or more stories}

## Parent Feature

{Feature issue link}

## Technical Requirements

- [ ] {Technical requirement}
- [ ] {Technical requirement}

## Stories Enabled

- {Story issue link}

## Acceptance Criteria

- [ ] {Validation criterion}
- [ ] {Validation criterion}
- [ ] Required tests are linked.

## Definition Of Done

- [ ] Implementation complete.
- [ ] Unit or integration tests added.
- [ ] Documentation updated when the seam affects future work.
- [ ] Root verification passes for Go changes.
```

## Test Issues

Create test issues in parallel with feature issues, not after implementation.

| Test issue | Parent feature | Estimate |
| --- | --- | --- |
| Test root scaffold and CLI scriptability | Prototype Isolation And Root Scaffold | 2 |
| Test control-plane desired/observed tracer | First Desired-State Resource Tracer | 3 |
| Test store migrations and filesystem layout | Store And Filesystem Guardrails | 3 |
| Test install planner scheduling and failures | Scriptable Install Planner | 3 |
| Test PHP and Composer runtime dependency behavior | PHP Runtime And Composer Tooling | 3 |
| Test daemon and supervisor process lifecycle | Daemon And Supervisor With Mailpit | 5 |
| Test database resource behavior | Stateful Database Resources | 5 |
| Test Redis, Mailpit, and RustFS resources | Cache, Mail, And Object Storage Resources | 5 |
| Test project contract and init generation | Project Contract And Init | 3 |
| Test link, env merge, and setup runner | Link, Env, And Setup | 5 |
| Test gateway, TLS, routes, and pv open | Gateway And pv open | 5 |
| Test Laravel helper command routing | Laravel Helper Commands | 3 |
| Test status UX and failure reporting | Desired And Observed Status UX | 3 |
| Test MVP/post-MVP scope guardrails | Post-MVP Backlog | 1 |

## Legacy Issue Disposition

- #96 should remain as the legacy PRD reference or be commented as superseded by
  this project plan. It should not be closed by scaffold work.
- #97-#113 should be commented as superseded by the new hierarchy before new
  execution begins.
- #114 and #115 should remain PR references. If reused, their PR bodies should
  remove accidental closure of #96.
