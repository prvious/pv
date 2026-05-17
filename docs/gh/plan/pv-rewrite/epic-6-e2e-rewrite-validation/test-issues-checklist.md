# Test Issues Checklist: Epic 6 - E2E Rewrite Validation

## Test Issue #221: Harness Isolation And Cleanup

Labels: `test`, `priority-critical`, `quality`, `e2e`, `ready-for-agent`

Blocked by: E6-EN1, E6-EN2, E6-EN3.

Required coverage:

- [ ] Harness builds or locates the active rewrite binary.
- [ ] Harness refuses to use legacy prototype binary.
- [ ] Sandbox HOME is under a temp directory.
- [ ] pv state, cache, config, data, and logs are under temp directories.
- [ ] Project fixture root is under a temp directory.
- [ ] Command runner captures argv, working directory, environment diff, stdout, stderr, exit code, elapsed time, and log paths.
- [ ] Cleanup removes sandbox-owned files and processes.
- [ ] Tests fail if real `~/.pv` would be used.

## Test Issue #225: Laravel Lifecycle E2E

Labels: `test`, `priority-critical`, `quality`, `e2e`, `laravel`, `ready-for-agent`

Blocked by: E6-T1, E6-S1, E6-S2, E6-S3.

Required coverage:

- [ ] `pv init` creates deterministic `pv.yml` with `version: 1`.
- [ ] `pv init` does not create or mutate `.env`.
- [ ] Existing `pv.yml` is not overwritten by default.
- [ ] Forced init overwrites deterministically.
- [ ] `pv link` validates `pv.yml` before durable state writes.
- [ ] `pv link` records project desired state.
- [ ] `.env` writes are declared-only and labeled.
- [ ] Setup commands run from project root with managed PHP first on `PATH`.
- [ ] Aggregate and targeted status work after link.
- [ ] `pv artisan`, `pv db`, `pv mail`, and `pv s3` route through current project state and declared resources.

## Test Issue #229: Failure And Recovery E2E

Labels: `test`, `priority-critical`, `quality`, `e2e`, `ready-for-agent`

Blocked by: E6-T1, E6-S4, E6-S5, E6-S6.

Required coverage:

- [ ] Missing runtime or resource install produces actionable error.
- [ ] Missing runtime or resource install appears in status as missing install or blocked.
- [ ] Setup failure stops subsequent setup commands.
- [ ] Setup failure records stderr, exit code, and next action.
- [ ] Runnable process crash records log path, last error, and next action.
- [ ] Gateway route, TLS, or DNS failure records actionable status without default host mutation.
- [ ] Corrective action changes blocked or failed status to healthy or expected pending state.
- [ ] Stale failure text does not remain after successful recovery.

## Test Issue #233: CI And Release Gate Behavior

Labels: `test`, `priority-high`, `quality`, `e2e`, `ready-for-agent`

Blocked by: E6-EN4, E6-EN5, E6-S7.

Required coverage:

- [ ] Tier 0 E2E command is documented.
- [ ] Tier 0 E2E command runs hermetic scenarios only.
- [ ] Tier 0 E2E exits non-zero when a scenario fails.
- [ ] Tier 0 E2E writes evidence with scenario, command, expected result, actual result, and log path.
- [ ] Tier 1 CI job runs only in GitHub-hosted CI VMs.
- [ ] Tier 2 CI job runs only in GitHub-hosted CI VMs.
- [ ] Tier 1 and Tier 2 refuse local execution.
- [ ] Tier 2 prints host actions before running.
- [ ] Release evidence template includes Tier 0, CI Tier 1, CI Tier 2, skipped tiers, and follow-up issue sections.

## Exit Evidence For All Epic 6 Test Issues

- [ ] Tests invoke the compiled active rewrite binary.
- [ ] Tests do not rely on private package internals for scenario assertions.
- [ ] Tests isolate `HOME` and do not use `t.Parallel()` with `t.Setenv`.
- [ ] Default tests do not download artifacts or mutate host DNS/TLS/browser state.
- [ ] Root verification passes for Go changes.
