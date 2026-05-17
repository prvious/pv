# Test Strategy: Epic 3 - Runtime, Daemon, And Resources

## Scope

Epic 3 tests cover:

- PHP runtime desired state and controller behavior;
- Composer dependency on PHP runtime;
- atomic, runtime-aware PHP and Composer shims;
- daemon reconciliation and wake-up behavior;
- resource-agnostic supervisor lifecycle;
- Mailpit as the first runnable resource;
- Postgres and MySQL stateful database resources;
- Redis cache resource;
- RustFS S3 resource;
- resource env values and secret redaction.

## Test Objectives

- Prove managed runtime behavior does not depend on system PHP.
- Prove Composer cannot reconcile without its required PHP runtime.
- Prove daemon and supervisor behavior is deterministic and testable with fake
  processes.
- Prove the supervisor remains resource-agnostic.
- Prove stateful resource data, logs, and status use canonical paths.
- Prove resource env values are explicit and not inferred from `.env`.
- Prove secret-like values are not printed in status or logs.

## ISTQB Techniques

| Technique | Epic 3 usage |
| --- | --- |
| Equivalence partitioning | Supported/unsupported runtime versions, installed/missing tools, declared/undeclared resources. |
| Boundary value analysis | Empty version, missing port, first restart, exhausted restart budget, empty credentials. |
| Decision table testing | Composer state across PHP present/missing/failed; resource state across installed/running/crashed/blocked. |
| State transition testing | desired -> pending -> running/ready/blocked/failed; process running -> crashed -> restarted -> exhausted. |
| Experience-based testing | Prior drift around system PHP fallback, daemon status gaps, and service-specific supervisor logic. |

## Test Matrix

| Area | Required tests |
| --- | --- |
| PHP runtime | Desired state, install planner integration, canonical runtime path, observed status states. |
| Composer | Required PHP version, missing-runtime blocked state, no system PHP fallback, shim command content. |
| Shims | Atomic replacement, permissions, failure cleanup, no ready status on partial writes. |
| Daemon | Desired-state enumeration, controller dispatch, wake/signal behavior, reconcile error persistence. |
| Supervisor | Start, stop, check, readiness, log path, restart budget, resource-agnostic package boundaries. |
| Mailpit | Desired state, process definition, SMTP/web ports, readiness, env values, observed status. |
| Postgres | Version line, data/log paths, process definition, readiness, env values, database commands, status states. |
| MySQL | Version line, initialization, socket/PID behavior, privileges, env values, commands, status states. |
| Redis | Version line, process flags, data/log paths, readiness, env values, status states. |
| RustFS | Credential model, API/console ports, data/log paths, S3 env values, secret redaction, status states. |

## Test Data

- Use `t.Setenv("HOME", t.TempDir())` for tests touching pv state.
- Do not use `t.Parallel()` with `t.Setenv` or global command/state mutation.
- Use fake artifact resolvers, installers, binaries, processes, clocks, and
  ports.
- Do not download real artifacts.
- Do not start real database or object storage processes in unit tests.
- Use deterministic secret-like values and assert they are redacted.

## Integration Coverage

Keep integration coverage narrow and opt-in where it needs real processes.

Minimum integration checks when resource implementations are ready:

1. PHP shim resolves to managed runtime path.
2. Composer blocked state appears when PHP is missing.
3. Mailpit fake or real process reaches ready status.
4. One database resource starts through the supervisor in a controlled temp
   state directory.
5. RustFS status redacts credentials.

## Verification Commands

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Exit Criteria

- All Epic 3 unit and integration tests pass.
- Supervisor resource-agnostic checks pass.
- Runtime and resource status covers healthy, missing install, blocked, stopped,
  crashed, and failed where applicable.
- No expensive artifact workflows were run.
- No status or log output prints secret-like values.
