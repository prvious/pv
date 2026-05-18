# Test Issues Checklist: Epic 3 - Runtime, Daemon, And Resources

## Test Issue #151: PHP And Composer Runtime Dependency Behavior

Labels: `test`, `priority-high`, `runtime`, `control-plane`, `ready-for-agent`

Required coverage:

- [ ] PHP desired state persists requested version.
- [ ] PHP controller uses canonical runtime paths.
- [ ] PHP controller integrates with install planner fakes.
- [ ] PHP observed status covers missing, pending, ready, and failed.
- [ ] Composer desired state records required PHP runtime.
- [ ] Composer reports blocked status when required PHP runtime is missing.
- [ ] Composer never falls back to system PHP.
- [ ] PHP and Composer shims are atomic and runtime-aware.
- [ ] Failed shim writes do not advertise ready status.

## Test Issue #156: Daemon And Supervisor Process Lifecycle

Labels: `test`, `priority-high`, `resource`, `control-plane`, `ready-for-agent`

Required coverage:

- [ ] Daemon enumerates desired resources.
- [ ] Daemon dispatches controllers through a testable registry.
- [ ] Durable state changes wake or signal daemon reconciliation.
- [ ] Reconcile errors become observed status.
- [ ] Supervisor starts, stops, checks, and reports readiness using fake processes.
- [ ] Restart budget behavior is deterministic.
- [ ] Supervisor API and tests contain no concrete resource names.
- [ ] Mailpit process definition includes SMTP port, web port, log path, readiness, and env values.
- [ ] Runnable observed status includes PID, port, log path, last error, and last reconcile time.

## Test Issue #161: Stateful Database Resource Behavior

Labels: `test`, `priority-high`, `resource`, `ready-for-agent`

Required coverage:

- [ ] Postgres desired state records version line.
- [ ] Postgres data and log paths use canonical helpers.
- [ ] Postgres process definition and readiness are tested with fakes.
- [ ] Postgres env values include host, port, username, password, version, and DSN where supported.
- [ ] Postgres `db:create`, `db:drop`, and `db:list` route to the declared resource.
- [ ] MySQL desired state records version line.
- [ ] MySQL initialization is explicit and idempotent where possible.
- [ ] MySQL socket/PID and privilege behavior are represented in tests.
- [ ] MySQL `db:create`, `db:drop`, and `db:list` route to the declared resource.
- [ ] Shared mechanics are extracted only after both resource tests prove the same shape.

## Test Issue #165: Redis, Mailpit, And RustFS Resource Behavior

Labels: `test`, `priority-high`, `resource`, `ready-for-agent`

Required coverage:

- [ ] Redis desired state records version line.
- [ ] Redis flags, data path, log path, readiness, env values, and status are tested.
- [ ] Mailpit remains explicit mail capture behavior, not generic HTTP-service behavior.
- [ ] RustFS credential model is explicit.
- [ ] RustFS API and console ports are represented.
- [ ] RustFS S3 env values render for declared env keys.
- [ ] Secret-like sentinel values do not appear in status or logs.
- [ ] No unit test starts real database, cache, mail, or object storage processes.

## Exit Evidence For All Epic 3 Test Issues

- [ ] Tests use fake installers, processes, ports, clocks, and artifact resolvers by default.
- [ ] Any real process integration is opt-in and documented in the PR body.
- [ ] Root verification passes.
