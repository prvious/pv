# Test Strategy: Epic 6 - E2E Rewrite Validation

## Scope

Epic 6 tests cover:

- E2E harness binary build and command runner behavior;
- sandbox isolation and cleanup;
- deterministic Laravel fixture generation;
- `pv init` lifecycle;
- `pv link`, declared env writes, setup commands, and signal ordering;
- aggregate and targeted status views;
- Laravel helper command routing;
- missing install, blocked dependency, setup failure, process crash, gateway
  failure, and recovery workflows;
- CI/release gate execution and evidence output.

## Quality Objectives

- Tier 0 E2E runs are hermetic and repeatable.
- 100% of MVP E2E scenarios use the compiled active rewrite binary.
- Local E2E does not touch real `HOME`, real `~/.pv`, DNS, TLS trust, browser,
  network artifact downloads, or live resources.
- Every failure E2E includes a recovery or next-action validation.
- E2E results include enough evidence to diagnose failures without rerunning
  immediately.

## ISTQB Techniques

| Technique | Epic 6 usage |
| --- | --- |
| Equivalence partitioning | Fresh project, existing project, linked project, declared resource, missing resource, failed resource. |
| Boundary value analysis | Empty setup list, one setup command, first command failure, missing `pv.yml`, duplicate aliases, no observed status. |
| Decision table testing | Link outcomes across missing installs, setup commands, env declarations, gateway declarations, and daemon availability. |
| State transition testing | uninitialized -> initialized -> linked -> reconciled; blocked -> corrected -> healthy; running -> crashed -> recovered. |
| Experience-based testing | Prior prototype risks: hidden `.env` inference, status drift, setup clobbering, daemon/process failures, host mutation. |

## Test Types Coverage Matrix

| Test type | Coverage |
| --- | --- |
| Functional | Full CLI workflows for init, link, status, helpers, failures, and recovery. |
| Non-functional | Hermetic isolation, deterministic cleanup, timeout guardrails, no default host mutation. |
| Structural | Harness proves it invokes the binary rather than private package APIs. |
| Change-related | Regression E2E for hidden env inference, setup fail-fast, status next actions, and resource redaction. |

## ISO 25010 Quality Priorities

| Characteristic | Priority | Validation |
| --- | --- | --- |
| Functional suitability | Critical | E2E scenarios cover MVP user workflows. |
| Reliability | Critical | Failure and recovery scenarios validate daemon, process, setup, and gateway behavior. |
| Security | High | Local E2E avoids host mutation, CI host checks run only on disposable GitHub VMs, and output verifies secret redaction. |
| Maintainability | High | Harness helpers are reusable and scenario evidence is structured. |
| Portability | High | Tier boundaries separate hermetic checks from host-specific checks. |
| Performance efficiency | Medium | Tier 0 has timeout guardrails and avoids expensive downloads. |
| Compatibility | Medium | E2E isolates HOME and project files to coexist with developer machines. |
| Usability | Medium | Failure evidence includes next actions and log paths. |

## Test Environment Strategy

| Environment | Purpose |
| --- | --- |
| Tier 0 hermetic | Required local-safe and CI E2E gate using temp HOME, fake artifacts, fake processes, and fake host adapters. |
| Tier 1 CI local process | GitHub CI VM daemon/supervisor checks with temp data roots and allocated ports; refuses local execution. |
| Tier 2 CI privileged host | GitHub CI VM DNS, TLS trust, and browser behavior checks; refuses local execution. |

## Test Data Strategy

- Generate minimal Laravel fixtures in temp project roots.
- Use `t.Setenv("HOME", t.TempDir())` for tests touching pv state.
- Do not use `t.Parallel()` in tests that call `t.Setenv` or mutate command/global state.
- Use deterministic secret-like sentinel values and assert they are absent from output.
- Use fake artifact catalogs and fake process binaries in Tier 0.
- Allocate ports through the harness; never hardcode shared ports in default E2E.

## Required E2E Scenario Matrix

| Scenario | Tier | Required assertions |
| --- | --- | --- |
| Harness isolation | 0 | Binary path, temp HOME, temp store, temp project, cleanup, captured output. |
| Init fresh Laravel project | 0 | Deterministic `pv.yml`, `version: 1`, no `.env` mutation, stdout/stderr behavior. |
| Init overwrite refusal | 0 | Existing `pv.yml` preserved, force overwrites deterministically. |
| Link declared project | 0 | Desired project state, declared env writes, setup execution, daemon signal ordering. |
| Aggregate and targeted status | 0 | Desired/observed output, log paths, next actions, redaction, targeted project/runtime/resource/gateway. |
| Helper commands | 0 | Artisan/db/mail/S3 route through current project and declared resources. |
| Missing install blocked state | 0 | Clear error, next action, status shows missing install or blocked. |
| Setup failure | 0 | Fail-fast, exit code, stderr, status failure evidence. |
| Runnable process crash | 0 or 1 | Crash observed, log path shown, next action present, recovery scenario follows. |
| Gateway failure | 0 or 2 | Route/TLS/DNS failure shown without default host mutation. |
| Recovery | 0 or 1 | Corrective action clears blocked/failed status. |
| Release gate | 0 | Required command exits non-zero on failure and writes evidence. |

## Verification Commands

Root verification for Go changes:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Epic 6 adds E2E jobs to `.github/workflows/tests.yml`. Tier 0 is safe locally and
in CI. Tier 1 and Tier 2 run in GitHub-hosted CI VMs and must refuse local laptop
execution.

## Exit Criteria

- Tier 0 E2E release gate passes locally and in CI.
- Tier 1 and Tier 2 CI jobs pass in GitHub-hosted VMs.
- Tier 1 and Tier 2 local refusal behavior is documented and tested.
- Every required scenario has evidence or a blocking issue.
- No default E2E test mutates real host state.
