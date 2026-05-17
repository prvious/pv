# QA Plan: Laravel-First Local Control Plane

## QA Principles

- Evidence before completion claims.
- Behavior is tested through public contracts, not private helper shape.
- Unit tests cover logic; integration tests cover module cooperation; E2E tests
  cover real OS behavior only where needed.
- Tests that touch pv state isolate `HOME`.
- No `t.Parallel()` in tests that call `t.Setenv` or mutate global state.
- Expensive artifact workflows are not run unless explicitly requested.

## Quality Gates

### Gate 1: Issue Ready

Entry criteria:

- [ ] Parent epic and feature are known.
- [ ] Acceptance criteria are testable.
- [ ] Dependencies are listed.
- [ ] Out-of-scope behavior is explicit.
- [ ] Test issue exists or test plan is linked.

Exit criteria:

- [ ] Implementation can start without re-discovering product intent.
- [ ] Agent can identify files/modules likely to change.

### Gate 2: Implementation Ready For Review

Entry criteria:

- [ ] Behavior is implemented.
- [ ] Focused tests were added or updated.
- [ ] `go-simplifier` was run for changed Go code before commit.

Exit criteria:

- [ ] `gofmt -w .` run.
- [ ] `go vet ./...` passes.
- [ ] `go build ./...` passes.
- [ ] `go test ./...` passes.
- [ ] Prototype verification passes if prototype files changed.
- [ ] PR body lists exact test commands.

### Gate 3: Feature Acceptance

Entry criteria:

- [ ] PR is reviewed.
- [ ] Tests pass.
- [ ] Feature issue acceptance criteria are checked.

Exit criteria:

- [ ] Status/output behavior is documented if user-facing.
- [ ] No hidden `.env`, service, setup, or migration behavior was introduced.
- [ ] Any deferred work is added to the post-MVP backlog.

### Gate 4: MVP Release Readiness

Entry criteria:

- [ ] All P0 features complete.
- [ ] P1 features required for MVP complete or explicitly deferred.
- [ ] E2E path exists for a fresh Laravel app.

Exit criteria:

- [ ] Fresh Laravel app initializes, links, and serves at HTTPS `.test`.
- [ ] Status shows desired and observed state.
- [ ] Logs are available for gateway and declared services.
- [ ] Post-MVP backlog is complete.
- [ ] Known limitations are documented.

## Metrics

| Metric | Target |
| --- | --- |
| Acceptance criteria coverage | 100% by test or manual QA |
| P0 feature verification | 100% automated where practical |
| Defect escape rate | Less than 5% of completed stories reopened |
| Root verification compliance | 100% for Go PRs |
| Hidden magic regression count | 0 |
| Supervisor resource coupling | 0 resource-specific names in supervisor API |

## Regression Focus

The old prototype history created these known risk areas:

- Service env updates touching unrelated projects.
- Binary services not binding to pre-linked projects.
- Daemon status drift after process startup failures.
- Permission mismatches in daemon-owned config directories.
- Hidden `.env` service inference.
- Hardcoded setup behavior during link.

Each related rewrite feature must include regression tests or an explicit note
explaining why the rewrite architecture makes the old failure impossible.

## Manual QA Checklist

Use for milestone-level validation:

- [ ] Run `pv init` in a Laravel project.
- [ ] Inspect generated `pv.yml`.
- [ ] Run `pv link`.
- [ ] Confirm `.env` contains only declared pv-managed writes.
- [ ] Start daemon.
- [ ] Visit `https://<app>.test`.
- [ ] Run `pv status`.
- [ ] Stop a managed process and confirm reconcile/status behavior.
- [ ] Check logs for gateway and declared services.
- [ ] Run a Laravel helper command through pinned PHP.

## Issue Hygiene

- Use one issue per story, enabler, or test work item.
- Do not pack multiple resource families into one story unless the issue is
  specifically about shared mechanics.
- Link test issues to the implementation issue they validate.
- Link PRs to the smallest completed issue.
- Do not use `Closes #96` for implementation PRs; #96 is legacy PRD reference.
