# Issues Checklist: Epic 5 - Status, Quality, And Scope Control

Create these issues after Epic 4 is published.

## Published Issues

Milestone: `pv rewrite MVP`

| Type | Issue | Title |
| --- | --- | --- |
| Epic | #193 | Epic: Status, Quality, And Scope Control |
| Feature | #194 | Feature: Desired And Observed Status UX |
| Feature | #195 | Feature: Post-MVP Backlog |
| Enabler | #196 | Enabler: Add Aggregate Status Model |
| User Story | #197 | User Story: Show Desired And Observed Resource Status |
| User Story | #198 | User Story: Show Failures Logs And Next Actions |
| User Story | #199 | User Story: Keep Status Output Scriptable |
| User Story | #200 | User Story: Support Targeted Status Views |
| Test | #201 | Test: Status UX Across Resource States |
| Enabler | #202 | Enabler: Create Post-MVP Backlog Document |
| User Story | #203 | User Story: Record Deferral Reasons And Reconsideration Triggers |
| Enabler | #204 | Enabler: Add MVP Scope Checklist To Planning Docs |
| Test | #205 | Test: Scope Guardrail And Backlog Completeness |

Tracker hygiene performed:

- Legacy flat issues #109 and #113 remain reference-only.
- Added superseded/reference comments to #109 and #113.
- Added `ready-for-agent` to Epic 5 leaf issues #196-#205.
- Updated Epic 5 container issue bodies with child issue links.

## Epic Issue

### Title

`Epic: Status, Quality, And Scope Control`

### Labels

`epic`, `priority-high`, `value-high`, `quality`, `control-plane`

### Body

```markdown
## Epic Description

Add aggregate status UX, release quality gates, and post-MVP scope control for
the Laravel-first rewrite.

Legacy references: #109, #113.

## Business Value

- Users can understand desired state, observed state, failures, logs, and next
  actions.
- Maintainers get explicit QA gates before MVP release.
- Deferred capabilities stay visible without expanding MVP scope.

## Features

- [ ] Feature: Desired And Observed Status UX
- [ ] Feature: Post-MVP Backlog

## Acceptance Criteria

- [ ] `pv status` reports desired state, observed state, failures, logs, and next actions.
- [ ] Status covers healthy, stopped, missing install, blocked, crashed, failed, and partially reconciled states where applicable.
- [ ] Status output remains stable and scriptable.
- [ ] Targeted status views are available where useful.
- [ ] Post-MVP backlog records omitted capabilities.
- [ ] Backlog entries include deferral reasons and reconsideration triggers.
- [ ] MVP scope checklist is part of planning or review flow.
- [ ] Final QA maps MVP acceptance criteria to evidence.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Root verification passes.
- [ ] Final QA evidence is documented.
```

## Feature Issues

### Feature: Desired And Observed Status UX

**Labels:** `feature`, `priority-critical`, `value-high`, `quality`, `control-plane`

```markdown
## Feature Description

Add aggregate and targeted status views that show desired state, observed state,
failures, logs, and next actions across projects, runtimes, resources, daemon,
supervisor, and gateway.

## Parent Epic

Epic: Status, Quality, And Scope Control

## Stories And Enablers

- [ ] Enabler: Add Aggregate Status Model
- [ ] User Story: Show Desired And Observed Resource Status
- [ ] User Story: Show Failures Logs And Next Actions
- [ ] User Story: Keep Status Output Scriptable
- [ ] User Story: Support Targeted Status Views
- [ ] Test: Status UX Across Resource States

## Dependencies

Blocked by:

- Epics 1-4 status providers and controllers

Blocks:

- MVP release readiness

## Acceptance Criteria

- [ ] Aggregate status includes projects, runtimes, tools, resources, gateway, daemon, and supervisor where applicable.
- [ ] Status normalizes healthy, stopped, missing install, blocked, crashed, failed, partial, and unknown states.
- [ ] Status includes log path, last error, last reconcile time, and next action where available.
- [ ] Output is stable and scriptable.
```

### Feature: Post-MVP Backlog

**Labels:** `feature`, `priority-high`, `value-high`, `quality`

```markdown
## Feature Description

Create the post-MVP backlog and scope guardrails that keep omitted capabilities
visible but outside MVP execution.

## Parent Epic

Epic: Status, Quality, And Scope Control

## Stories And Enablers

- [ ] Enabler: Create Post-MVP Backlog Document
- [ ] User Story: Record Deferral Reasons And Reconsideration Triggers
- [ ] Enabler: Add MVP Scope Checklist To Planning Docs
- [ ] Test: Scope Guardrail And Backlog Completeness

## Dependencies

Blocked by:

- Rewrite PRD and MVP scope decisions

## Acceptance Criteria

- [ ] Backlog document exists.
- [ ] Omitted capabilities from PRD and planning are listed.
- [ ] Each backlog item has a deferral reason.
- [ ] Each backlog item has a reconsideration trigger.
- [ ] Scope checklist is available during review.
```

## Story And Enabler Issues

### Enabler: Add Aggregate Status Model

**Labels:** `enabler`, `priority-critical`, `quality`, `control-plane`

```markdown
## Enabler Description

Add the aggregate status model and provider shape for project, runtime, resource,
gateway, daemon, and supervisor status.

## Acceptance Criteria

- [ ] Model includes desired and observed state.
- [ ] Model includes log path, last error, last reconcile time, and next action.
- [ ] Provider shape lets controllers expose status without rendering.
- [ ] Secret-like values are redacted.
```

### User Story: Show Desired And Observed Resource Status

**Labels:** `user-story`, `priority-critical`, `quality`, `control-plane`

```markdown
## Story Statement

As a Laravel developer, I want `pv status` to show what should exist and what
actually exists so that I can understand drift.

## Acceptance Criteria

- [ ] Status shows desired state.
- [ ] Status shows observed state.
- [ ] Status distinguishes healthy, stopped, missing install, blocked, crashed, failed, partial, and unknown states where applicable.
- [ ] Status includes project and resource context.
```

### User Story: Show Failures Logs And Next Actions

**Labels:** `user-story`, `priority-critical`, `quality`

```markdown
## Story Statement

As a Laravel developer, I want status output to include failures, log paths, and
next actions so that I can recover quickly.

## Acceptance Criteria

- [ ] Status includes last error when available.
- [ ] Status includes log path when available.
- [ ] Status includes next action for blocked or failed states.
- [ ] Status does not print secret-like values.
```

### User Story: Keep Status Output Scriptable

**Labels:** `user-story`, `priority-critical`, `quality`

```markdown
## Story Statement

As a maintainer, I want status output to remain stable and scriptable so that pv
can be used in automation.

## Acceptance Criteria

- [ ] Human status format is stable.
- [ ] stdout remains pipeable where command output is designed for piping.
- [ ] Human progress/status goes to stderr where applicable.
- [ ] Machine-readable output is explicit if introduced.
```

### User Story: Support Targeted Status Views

**Labels:** `user-story`, `priority-high`, `quality`

```markdown
## Story Statement

As a Laravel developer, I want targeted status views so that I can inspect one
project, runtime, resource, or gateway without scanning all output.

## Acceptance Criteria

- [ ] Targeted views reuse aggregate status data.
- [ ] Missing target errors are actionable.
- [ ] Targeted views do not hide failures relevant to the target.
- [ ] Output remains stable.
```

### Test: Status UX Across Resource States

**Labels:** `test`, `priority-high`, `quality`, `control-plane`

```markdown
## Test Description

Validate aggregate and targeted status behavior across representative resource
and project states.

## Acceptance Criteria

- [ ] Tests cover healthy, stopped, missing install, blocked, crashed, failed, partial, and unknown states.
- [ ] Tests cover failures, log paths, last reconcile time, and next actions.
- [ ] Tests cover secret redaction.
- [ ] Tests cover scriptable output behavior.
```

### Enabler: Create Post-MVP Backlog Document

**Labels:** `enabler`, `priority-high`, `quality`

```markdown
## Enabler Description

Create the post-MVP backlog document under the rewrite planning package.

## Acceptance Criteria

- [ ] Backlog document exists.
- [ ] Omitted capabilities from PRD and planning are listed.
- [ ] Backlog entries are not treated as MVP implementation tasks.
- [ ] Document location is referenced from the planning README.
```

### User Story: Record Deferral Reasons And Reconsideration Triggers

**Labels:** `user-story`, `priority-high`, `quality`

```markdown
## Story Statement

As a maintainer, I want each deferred capability to include a reason and trigger
so that future scope changes are deliberate.

## Acceptance Criteria

- [ ] Every backlog item has a deferral reason.
- [ ] Every backlog item has a reconsideration trigger.
- [ ] Triggers are specific enough to guide future planning.
- [ ] MVP issues do not require deferred items.
```

### Enabler: Add MVP Scope Checklist To Planning Docs

**Labels:** `enabler`, `priority-high`, `quality`

```markdown
## Enabler Description

Add a concise MVP scope checklist to planning or review docs.

## Acceptance Criteria

- [ ] Checklist is easy to apply during PR review.
- [ ] Checklist asks whether new work is in MVP scope.
- [ ] Checklist points deferred work to the post-MVP backlog.
- [ ] Checklist is referenced from relevant planning docs.
```

### Test: Scope Guardrail And Backlog Completeness

**Labels:** `test`, `priority-high`, `quality`

```markdown
## Test Description

Validate that post-MVP backlog and MVP scope guardrails are complete enough for
release planning.

## Acceptance Criteria

- [ ] QA check maps PRD out-of-scope items to backlog entries.
- [ ] QA check confirms each backlog item has reason and trigger.
- [ ] QA check confirms MVP checklist exists.
- [ ] QA check confirms no MVP issue depends on deferred items.
```
