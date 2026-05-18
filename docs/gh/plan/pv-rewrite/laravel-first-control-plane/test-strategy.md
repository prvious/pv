# Test Strategy: Laravel-First Local Control Plane

## Testing Scope

The test strategy covers the rewrite MVP:

- Root rewrite module and prototype isolation.
- Desired-state store and observed-status model.
- Commands that request state changes.
- Controllers that reconcile resource families.
- Resource-agnostic supervisor and daemon reconcile loop.
- Managed PHP, Composer, Postgres, MySQL, Redis, Mailpit, and RustFS.
- Laravel `pv.yml` contract, init, link, env merge, and setup runner.
- Gateway `.test` HTTPS routing and `pv open`.
- Status UX and post-MVP scope guardrails.

## Quality Objectives

- 100% of acceptance criteria have at least one test or documented manual QA
  check.
- Commands keep stdout pipeable and write human status to stderr.
- Tests prove desired state and observed status are separate.
- Tests prove `pv link` does not infer services from `.env`.
- Supervisor tests prove no resource-specific behavior leaks into process
  lifecycle code.
- E2E coverage is narrow and reserved for OS integration that unit tests cannot
  prove.

## ISTQB Test Design Techniques

| Technique | Use in this project |
| --- | --- |
| Equivalence partitioning | Valid/invalid versions, service names, aliases, command args, contract fields. |
| Boundary value analysis | Empty config, missing service version, duplicate aliases, zero setup commands, failed first setup command. |
| Decision table testing | `pv link` behavior across declared services, missing installs, env declarations, setup presence. |
| State transition testing | Desired -> pending -> ready/blocked/failed; daemon process running -> crashed -> restarted -> exhausted. |
| Experience-based testing | Prior prototype bugs: env clobbering, daemon permission failures, service status drift, hidden setup magic. |

## Test Types Coverage Matrix

| Test type | Coverage |
| --- | --- |
| Functional | Commands, contract parsing, controller reconciliation, service env, setup runner, helper routing. |
| Non-functional | Startup reliability, restart budget, bounded downloads, scriptable output, path safety. |
| Structural | Package role boundaries, supervisor resource-agnostic tests, store migration seams. |
| Change-related | Regression tests for old `.env` clobbering, daemon status, service binding, setup inference. |

## ISO 25010 Quality Priorities

| Characteristic | Priority | Validation |
| --- | --- | --- |
| Functional suitability | Critical | Acceptance tests for every feature and story. |
| Reliability | Critical | Daemon restart, supervisor crash handling, status after failures. |
| Maintainability | Critical | Module-boundary tests and fake adapters for controllers. |
| Security | High | Secrets not logged, file permissions for state and credentials, no hidden env writes. |
| Portability | High | Host adapters for macOS now, Linux/Windows deferred but not blocked by hardcoded assumptions. |
| Performance efficiency | Medium | Bounded downloads, no unbounded process loops, reasonable status latency. |
| Compatibility | Medium | Coexistence with user `.env`, PATH shims, and existing project files. |
| Usability | Medium | Clear errors, next actions, status output, no TUI-only paths. |

## Test Environment Strategy

| Environment | Purpose |
| --- | --- |
| Unit test temp dirs | Store, path, contract, env, setup, installer, controller behavior. |
| Fake host adapters | DNS, TLS, browser open, process signal behavior without OS mutation. |
| Fake binaries | Supervisor lifecycle, readiness, restart budget. |
| Local integration | Real process supervision when required. |
| CI E2E | Fresh Laravel app, daemon startup, HTTPS `.test`, selected real resources. |

## Test Data Strategy

- Use `t.TempDir()` for filesystem state.
- Use `t.Setenv("HOME", t.TempDir())` for tests touching pv state.
- Do not use `t.Parallel()` with `t.Setenv` or global command/state mutation.
- Use deterministic clocks for observed status timestamps.
- Use generated fake projects with minimal Laravel markers instead of large app
  fixtures where possible.
- Store secret-like test values only in temp files and assert they are not
  printed.

## Required Verification Commands

Root rewrite:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Prototype reference, only if prototype files changed:

```bash
cd legacy/prototype
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Feature Test Matrix

| Feature | Required test focus |
| --- | --- |
| Prototype Isolation And Root Scaffold | Root/prototype buildability, CLI scriptability, no Fang by default. |
| First Desired-State Resource Tracer | Desired/observed persistence, command state write, controller status write. |
| Store And Filesystem Guardrails | Layout families, state permissions, schema version, applied migrations. |
| Scriptable Install Planner | Dependency order, bounded parallelism, atomic shims, failure rollback. |
| PHP Runtime And Composer Tooling | Runtime resolution, missing runtime blocked state, shim behavior. |
| Daemon And Supervisor With Mailpit | Process lifecycle, readiness, restart budget, daemon signal, observed status. |
| Stateful Database Resources | Version lines, data dirs, process definitions, env values, explicit DB commands. |
| Cache, Mail, And Object Storage Resources | Redis flags, Mailpit ports, RustFS credentials/routes, env values, status. |
| Project Contract And Init | Schema parse, validation, Laravel detection, generated defaults, overwrite rules. |
| Link, Env, And Setup | No `.env` inference, managed labels, removed declarations, setup fail-fast. |
| Gateway And pv open | Host generation, aliases, TLS SANs, route rendering, browser-open adapter. |
| Laravel Helper Commands | Current project resolution, pinned PHP, declared-resource routing, missing-resource errors. |
| Desired And Observed Status UX | Healthy, stopped, missing install, blocked, crashed, partial state output. |
| Post-MVP Backlog | Every omitted capability listed with deferral reason and trigger. |

## E2E Strategy

Epic 6 owns the rewrite E2E strategy. E2E tests are added after unit,
controller, and status coverage exists, and the required default E2E tier is
hermetic.

Minimum MVP E2E:

1. Generate a fresh Laravel project fixture.
2. Run `pv init`.
3. Review generated `pv.yml`.
4. Run `pv link`.
5. Start daemon.
6. Verify app responds at `https://<app>.test`.
7. Verify `pv status` reports desired and observed state.
8. Verify logs are accessible for gateway and declared services.

Service-specific E2E should be narrow:

- Postgres: install/start/create DB/env/status.
- MySQL: install/start/create DB/env/status.
- Redis: install/start/env/status.
- Mailpit: install/start/SMTP/web/status.
- RustFS: install/start/S3 env/console/status.

See `../epic-6-e2e-rewrite-validation/test-strategy.md` for the executable E2E
scenario matrix, tiering model, and release gate.
