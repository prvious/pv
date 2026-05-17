# QA Plan: Epic 1 - Rewrite Foundation

## Quality Gates

### Gate 1: Ready For Implementation

- [ ] Labels exist in GitHub.
- [ ] Epic 1 issue hierarchy is created or staged from `issues-checklist.md`.
- [ ] Legacy issues #97-#99 and PR #114 are treated as references only.
- [ ] Scope excludes PHP, Composer, daemon, supervisor, Laravel, gateway, and
  backing resources.

### Gate 2: Ready For Review

- [ ] Prototype move is complete if included in the PR.
- [ ] Root scaffold is complete if included in the PR.
- [ ] First tracer behavior is complete if included in the PR.
- [ ] Focused tests are present.
- [ ] `go-simplifier` was run for changed Go code before commit.

### Gate 3: Acceptance

- [ ] Root verification passes.
- [ ] Prototype verification passes if prototype files changed.
- [ ] CLI stdout/stderr behavior is tested.
- [ ] Desired and observed state separation is tested.
- [ ] No implementation PR closes legacy #96.

## Manual Review Checklist

- [ ] Root contains active rewrite files only.
- [ ] `legacy/prototype` is visibly reference-only.
- [ ] New root code does not import prototype packages.
- [ ] No Fang dependency was added.
- [ ] The first tracer uses fake or marker installers in tests.
- [ ] Status output explains pending, ready, and failed states.

## Known Non-Goals

- No SQLite implementation yet unless it is deliberately pulled forward.
- No daemon or supervisor yet.
- No PHP or Composer implementation yet.
- No Laravel project contract yet.
- No gateway, DNS, or TLS behavior yet.
- No real artifact downloads in tests.
