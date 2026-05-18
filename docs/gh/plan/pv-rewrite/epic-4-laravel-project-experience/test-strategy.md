# Test Strategy: Epic 4 - Laravel Project Experience

## Scope

Epic 4 tests cover:

- versioned `pv.yml` parsing and validation;
- Laravel project detection and deterministic contract generation;
- `pv init` overwrite behavior;
- `pv link` project desired-state writes;
- managed `.env` merge behavior;
- setup runner PATH pinning and fail-fast behavior;
- gateway desired/observed state and route rendering;
- DNS, TLS, and browser adapters;
- helper command routing for Artisan, database, mail, and S3 workflows.

## Test Objectives

- Prove Laravel contracts are explicit, deterministic, and reviewable.
- Prove `pv link` never infers services or env values from `.env`.
- Prove user-authored `.env` values are preserved.
- Prove setup commands run through managed PHP, not system PHP.
- Prove gateway behavior is testable without unsafe OS mutation.
- Prove helper commands route through current project and declared resources.

## ISTQB Techniques

| Technique | Epic 4 usage |
| --- | --- |
| Equivalence partitioning | Valid/invalid contracts, Laravel/non-Laravel directories, declared/undeclared services. |
| Boundary value analysis | Empty aliases, duplicate aliases, empty setup list, existing empty `.env`, missing `pv.yml`. |
| Decision table testing | `pv link` behavior across missing installs, env declarations, setup presence, gateway declarations. |
| State transition testing | uninitialized -> initialized -> linked -> gateway ready/failed; setup pending -> passed/failed. |
| Experience-based testing | Prevent env clobbering, hidden `.env` inference, system PHP fallback, and unsafe host mutation. |

## Test Matrix

| Area | Required tests |
| --- | --- |
| Contract schema | Version, PHP, services, aliases, setup commands, unknown fields, unsupported values. |
| Laravel detection | Positive markers, missing markers, unsupported project fallback. |
| `pv init` | Deterministic YAML, no `.env` mutation, overwrite refusal, forced overwrite. |
| Project registry | Durable desired state, deterministic identity, store-before-signal ordering. |
| Env writer | Preserve user lines, managed labels, backup, update, removal, no `.env` inference. |
| Setup runner | Working directory, managed PATH, env propagation, streamed output, fail fast. |
| Gateway | Route rendering, primary host, aliases, TLS SANs, DNS adapter, supervisor process definition. |
| `pv open` | Current-project resolution, browser adapter call, missing link/gateway errors. |
| Helpers | Pinned PHP Artisan, declared DB/Mailpit/RustFS routing, missing-resource errors, secret redaction. |

## Test Data

- Use generated minimal Laravel project fixtures.
- Use `t.Setenv("HOME", t.TempDir())` for tests touching pv state.
- Do not use `t.Parallel()` with `t.Setenv` or global command/state mutation.
- Use fake env files with mixed user-authored and pv-managed keys.
- Use fake DNS, TLS, browser, process, and runtime adapters.
- Do not mutate real `/etc/hosts`, keychains, trust stores, or browsers in unit
  tests.

## Integration Coverage

Keep integration coverage narrow and opt-in where it needs real OS behavior.

Minimum integration checks when implementations are ready:

1. `pv init` creates deterministic `pv.yml` in a Laravel fixture.
2. `pv link` writes declared env values and preserves user entries.
3. Setup runner executes a fake Artisan command through managed PHP path.
4. Gateway renderer produces stable config for primary host and aliases.
5. `pv open` invokes the browser adapter with the expected URL.

## Verification Commands

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Test Issue Contract

Use `test-issues-checklist.md` as the execution checklist. Epic 4 has exactly
four test issues:

- #175 validates `version: 1` contracts, parser/generator behavior, and `pv init`.
- #181 validates `pv link`, declared env writes, setup runner, and store-before-signal ordering.
- #187 validates gateway route rendering, DNS/TLS/browser adapters, and `pv open`.
- #192 validates Artisan, database, mail, and S3 helper routing.

Tests must prove `pv link` never infers services or env values from `.env`.

## Exit Criteria

- All Epic 4 unit and integration tests pass.
- No test mutates real host DNS, TLS trust, or browser state.
- `.env` tests prove declared-only behavior.
- Helper tests prove commands route through declared project resources.
