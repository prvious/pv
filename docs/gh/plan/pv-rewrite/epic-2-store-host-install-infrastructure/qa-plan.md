# QA Plan: Epic 2 - Store, Host, And Install Infrastructure

## Quality Gates

### Gate 1: Ready For Implementation

- [ ] Epic 1 foundation issues are published.
- [ ] Store seam from Epic 1 is understood.
- [ ] Canonical filesystem layout is accepted.
- [ ] Install planner scope excludes real resource installation.
- [ ] Test issues are linked to feature issues.

### Gate 2: Ready For Review

- [ ] Path helper tests are present.
- [ ] Store schema/migration tests are present.
- [ ] Contract version decision is visible.
- [ ] Install planner tests use fake adapters.
- [ ] `go-simplifier` was run for changed Go code before commit.

### Gate 3: Acceptance

- [ ] Root verification passes.
- [ ] No tests require network or artifact downloads.
- [ ] Failed install paths do not signal reconciliation.
- [ ] Failed install paths do not expose shims or completed state.
- [ ] APIs are ready for Epic 3 runtime/resource implementation.

## Manual Review Checklist

- [ ] `~/.pv/bin` is shims/symlinks only.
- [ ] Real binaries live under runtime/tool/service roots.
- [ ] Data lives under `data`.
- [ ] Logs live under `logs`.
- [ ] State lives under `state`.
- [ ] Cache lives under `cache/artifacts`.
- [ ] No resource-specific path family is hardcoded outside host helpers.
- [ ] Store schema/migration naming is explicit.
- [ ] Install planner phases are clear: resolve, download, install, expose,
  persist, signal.

## Known Non-Goals

- No real PHP/Composer installs.
- No real service installs.
- No daemon implementation.
- No gateway behavior.
- No Laravel project contract parsing beyond contract-version decision.
- No artifact publishing workflows.
