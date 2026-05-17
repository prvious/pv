# Implementation Plan: Epic 5 - Status, Quality, And Scope Control

## Execution Rules

- Treat legacy issues #109 and #113 as reference only.
- Status must explain desired state, observed state, failures, logs, and next
  actions.
- Keep commands scriptable: return errors, keep stdout pipeable, and write human
  status to stderr unless a command explicitly defines otherwise.
- Keep omitted capabilities out of MVP issues.
- Use Go for repository logic.
- Before Go work, activate `golang-pro` and `modern-go`.
- Before each commit, run `go-simplifier` on changed Go code.
- Always try to add or update tests for changed behavior.
- Before handing off Go changes, run:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Implementation Contract

Execute the published leaf issues in dependency order and keep planning-file
changes separate from Go status implementation when possible.

| Issue range | Required output |
| --- | --- |
| #196-#201 | Aggregate status model, providers, rendering, targeted views, and status tests. |
| #202-#205 | `post-mvp-backlog.md`, `mvp-scope-checklist.md`, and backlog/scope QA checks. |

Non-negotiable decisions:

- Stacked diff branch is `rewrite/epic-5-status-quality-scope` and its base is
  `rewrite/epic-4-laravel-project-experience`.
- Epic 5 PRs do not target `main` directly.
- Status providers return data; they do not render UI.
- Normalized states are exactly `healthy`, `stopped`, `missing_install`, `blocked`, `crashed`, `failed`, `partial`, and `unknown`.
- Targeted status views are exactly `project`, `runtime`, `resource`, and `gateway` for MVP.
- Secret-like values are redacted from all human status output.
- Deferred backlog entries are not MVP implementation tasks.

## Suggested Package Ownership

- `internal/status` owns aggregate status models, state normalization, and
  scriptable status views.
- Resource and project packages own their own status providers.
- `internal/console` owns human rendering behavior.
- Planning docs own the post-MVP backlog and scope checklist unless a later
  task adds dedicated repo automation.

## Feature 5.1: Desired And Observed Status UX

**Goal:** Make pv explain what should exist, what exists, what failed, and what
to do next.

### Implementation Sequence

1. Add aggregate status model that can include projects, runtimes, tools,
   resources, gateway, daemon, and supervisor state.
2. Add status provider interfaces for controllers.
3. Normalize states: healthy, stopped, missing install, blocked, crashed,
   failed, partially reconciled, and unknown.
4. Add log path, last error, last reconcile time, and next action fields.
5. Add `pv status` aggregation.
6. Add targeted status views for `project`, `runtime`, `resource`, and `gateway`.
7. Define stable human rendering.
8. Define pipeable output only when intentionally supported.
9. Add tests across representative resource and project states.

### Acceptance Notes

- Status should help the user recover, not only describe failure.
- Avoid one-off status formatting inside resource packages.
- Keep secrets redacted.

## Feature 5.2: Post-MVP Backlog

**Goal:** Keep MVP scope explicit and prevent deferred capabilities from leaking
into active execution.

### Implementation Sequence

1. Maintain `docs/gh/plan/pv-rewrite/post-mvp-backlog.md`.
2. Populate omitted capabilities from the PRD and planning discussions.
3. For each deferred capability, record deferral reason and reconsideration
   trigger.
4. Maintain `docs/gh/plan/pv-rewrite/mvp-scope-checklist.md`.
5. Use test issue #205 to check backlog completeness and scope guardrails before
   release.

### Acceptance Notes

- Deferred does not mean forgotten.
- Backlog entries must not create implementation obligations for MVP.
- Scope checklist should be short enough to be used in reviews.

## Critical Path

1. Status provider shape.
2. Aggregate status model.
3. `pv status` rendering and targeted views.
4. Status tests across resource states.
5. Post-MVP backlog doc.
6. MVP scope checklist.
7. Final QA evidence mapping.

## Review Checklist

- [ ] Status includes desired and observed state.
- [ ] Status includes failures, log paths, and next actions.
- [ ] Status redacts secret-like values.
- [ ] Human output is stable and scriptable.
- [ ] Targeted views reduce noise without hiding aggregate state.
- [ ] Every backlog item has a deferral reason.
- [ ] Every backlog item has a reconsideration trigger.
- [ ] Final QA maps acceptance criteria to evidence.
