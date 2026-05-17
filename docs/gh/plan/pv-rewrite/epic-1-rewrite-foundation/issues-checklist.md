# Issues Checklist: Epic 1 - Rewrite Foundation

Create these issues after labels are available.

## Published Issues

Milestone: `pv rewrite MVP`

| Type | Issue | Title |
| --- | --- | --- |
| Epic | #116 | Epic: Rewrite Foundation |
| Feature | #117 | Feature: Prototype Isolation And Root Scaffold |
| Feature | #118 | Feature: First Desired-State Resource Tracer |
| Enabler | #119 | Enabler: Move Prototype Into legacy/prototype |
| Enabler | #120 | Enabler: Scaffold Active Root Go Module |
| User Story | #121 | User Story: Minimal Scriptable CLI |
| Test | #122 | Test: Root/Prototype Build And CLI Checks |
| Enabler | #123 | Enabler: Add Desired And Observed Store Seam |
| User Story | #124 | User Story: Request Installable Resource Desired State |
| User Story | #125 | User Story: Reconcile First Installable Resource |
| User Story | #126 | User Story: Report Desired And Observed Status |
| Test | #127 | Test: Control-Plane Tracer Behavior |

Tracker hygiene performed:

- Removed `ready-for-agent` from legacy flat issues #96-#113.
- Added superseded/reference comments to #96-#99.
- Added `ready-for-agent` to Epic 1 leaf issues #119-#127.

## Epic Issue

### Title

`Epic: Rewrite Foundation`

### Labels

`epic`, `priority-critical`, `value-high`, `control-plane`

### Body

```markdown
## Epic Description

Establish the active rewrite workspace and prove the first control-plane
vertical slice.

Legacy references: #97, #98, #99, #114.

## Business Value

- Future rewrite work starts from a clean root module.
- The prototype remains buildable as reference-only code.
- The first desired-state tracer proves the architecture before resource
  complexity increases.

## Features

- [ ] Feature: Prototype Isolation And Root Scaffold
- [ ] Feature: First Desired-State Resource Tracer

## Acceptance Criteria

- [ ] Prototype is isolated under `legacy/prototype`.
- [ ] Root module builds independently.
- [ ] Root CLI has minimal scriptable help/version/error behavior.
- [ ] First tracer proves command -> desired state -> controller -> observed
  status.
- [ ] Status distinguishes pending, ready, and failed observed states.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Root verification passes.
- [ ] Prototype verification passes if prototype files changed.
- [ ] No PR closes legacy #96.
```

## Feature Issues

### Feature: Prototype Isolation And Root Scaffold

**Labels:** `feature`, `priority-critical`, `value-high`, `control-plane`

```markdown
## Feature Description

Move the prototype into a reference-only module and scaffold the root as the
active rewrite module with minimal scriptable CLI behavior.

## Parent Epic

Epic: Rewrite Foundation

## Stories And Enablers

- [ ] Enabler: Move prototype into `legacy/prototype`
- [ ] Enabler: Scaffold active root Go module
- [ ] User Story: Minimal scriptable CLI
- [ ] Test: Root/prototype build and CLI checks

## Dependencies

Blocked by: none.

Blocks:

- Feature: First Desired-State Resource Tracer

## Acceptance Criteria

- [ ] Old Go module builds from `legacy/prototype`.
- [ ] Root no longer contains old command tree or implementation packages.
- [ ] Root contains fresh active Go module.
- [ ] `help`, `version`, and unknown command behavior are tested.
- [ ] Fang is not added by default.

## Definition Of Done

- [ ] Root verification passes.
- [ ] Prototype verification passes.
- [ ] Active versus reference code boundaries are documented.
```

### Feature: First Desired-State Resource Tracer

**Labels:** `feature`, `priority-critical`, `value-high`, `control-plane`

```markdown
## Feature Description

Build the first small control-plane tracer around one installable resource. The
slice proves command -> desired state -> controller -> observed status without
taking on Laravel, daemon, or service complexity.

## Parent Epic

Epic: Rewrite Foundation

## Stories And Enablers

- [ ] Enabler: Add desired and observed store seam
- [ ] User Story: Request installable resource desired state
- [ ] User Story: Reconcile first installable resource
- [ ] User Story: Report desired and observed status
- [ ] Test: Control-plane tracer behavior

## Dependencies

Blocked by:

- Feature: Prototype Isolation And Root Scaffold

Blocks:

- Epic 2: Store, Host, And Install Infrastructure
- Epic 3: Runtime, Daemon, And Resources

## Acceptance Criteria

- [ ] Command validates a user request and records desired state.
- [ ] Command does not install directly.
- [ ] Controller reconciles desired state for one installable resource.
- [ ] Observed status is stored separately from desired state.
- [ ] Status reports pending, ready, and failed states.

## Definition Of Done

- [ ] Command, store, controller, and status tests are present.
- [ ] Root verification passes.
- [ ] No real artifact downloads are required for tests.
```

## Story And Enabler Issues

### Enabler: Move Prototype Into `legacy/prototype`

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Move the current prototype implementation into `legacy/prototype` as a buildable
reference module.

## Acceptance Criteria

- [ ] Old Go module builds from `legacy/prototype`.
- [ ] Root old `cmd`, `internal`, and prototype `main.go` are moved.
- [ ] Prototype-specific docs move with the prototype when they describe old
  behavior.
- [ ] New root rewrite code does not import prototype packages.

## Definition Of Done

- [ ] Prototype verification passes.
- [ ] Root status is ready for a fresh rewrite module.
```

### Enabler: Scaffold Active Root Go Module

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Create the root Go module and active entrypoint for the rewrite.

## Acceptance Criteria

- [ ] Root `go.mod` exists.
- [ ] Root `main.go` exists.
- [ ] CLI package has a testable `Run(args, stdout, stderr)` style seam.
- [ ] Root build passes.
```

### User Story: Minimal Scriptable CLI

**Labels:** `user-story`, `priority-critical`, `control-plane`

```markdown
## Story Statement

As an automation user, I want the root CLI to have predictable help, version,
and usage errors so that scripts can rely on stable behavior from the start.

## Acceptance Criteria

- [ ] `help` writes usage to stdout.
- [ ] `version` writes pipeable version output to stdout.
- [ ] Unknown commands write a useful message to stderr.
- [ ] Unknown commands return a usage error.
- [ ] Fang is not added by default.
```

### Enabler: Add Desired And Observed Store Seam

**Labels:** `enabler`, `priority-critical`, `control-plane`

```markdown
## Enabler Description

Add the small store seam needed by the first tracer. The implementation can be
simple, but desired state and observed status must be separate.

## Acceptance Criteria

- [ ] Desired resource state can be written and read.
- [ ] Observed status can be written and read.
- [ ] Desired writes do not create observed status.
- [ ] Observed writes do not mutate desired state.
- [ ] Invalid versions are rejected.
```

### User Story: Request Installable Resource Desired State

**Labels:** `user-story`, `priority-critical`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want a command to request installation of one small resource
by writing desired state so that command behavior stays thin and testable.

## Acceptance Criteria

- [ ] Command validates resource version.
- [ ] Command writes desired state.
- [ ] Command does not install directly.
- [ ] Command writes human status to stderr.
```

### User Story: Reconcile First Installable Resource

**Labels:** `user-story`, `priority-critical`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want a controller to reconcile one installable resource so
that the control-plane loop is proven before adding real services.

## Acceptance Criteria

- [ ] Controller reads desired state.
- [ ] Controller no-ops when desired state is absent.
- [ ] Controller writes ready observed status on success.
- [ ] Controller writes failed observed status and next action on failure.
- [ ] Tests use fake installers, not real downloads.
```

### User Story: Report Desired And Observed Status

**Labels:** `user-story`, `priority-critical`, `control-plane`

```markdown
## Story Statement

As a maintainer, I want status output for the first tracer so that desired state
and observed state are visible separately.

## Acceptance Criteria

- [ ] Status shows desired state when present.
- [ ] Status shows pending when observed status is absent.
- [ ] Status shows ready state.
- [ ] Status shows failed state with next action.
- [ ] Human status is written to stderr.
```

## Test Issues

### Test: Root/Prototype Build And CLI Checks

**Labels:** `test`, `priority-high`, `control-plane`

```markdown
## Test Scope

Validate Feature: Prototype Isolation And Root Scaffold.

## Test Cases

- [ ] Root module builds.
- [ ] Prototype module builds from `legacy/prototype`.
- [ ] CLI help output is stable.
- [ ] CLI version output is on stdout.
- [ ] Unknown command behavior returns usage error.
- [ ] Fang dependency is absent unless explicitly justified.

## Definition Of Done

- [ ] Tests are committed with implementation.
- [ ] Root verification passes.
- [ ] Prototype verification passes.
```

### Test: Control-Plane Tracer Behavior

**Labels:** `test`, `priority-high`, `control-plane`

```markdown
## Test Scope

Validate Feature: First Desired-State Resource Tracer.

## Test Cases

- [ ] Desired state persists.
- [ ] Observed status persists separately.
- [ ] Command writes desired state only.
- [ ] Controller records ready status.
- [ ] Controller records failed status and next action.
- [ ] Status reports pending, ready, and failed states.

## Definition Of Done

- [ ] Tests are committed with implementation.
- [ ] No real artifact downloads are needed.
- [ ] Root verification passes.
```
