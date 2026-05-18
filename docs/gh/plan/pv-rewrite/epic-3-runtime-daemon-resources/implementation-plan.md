# Implementation Plan: Epic 3 - Runtime, Daemon, And Resources

## Execution Rules

- Treat legacy issues #100-#105 and PR #115 as reference only.
- Commands request desired state; controllers reconcile resources.
- The supervisor only starts, stops, checks, and records processes.
- Resource packages own resource-specific flags, readiness, env values, and
  commands.
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

Execute the published leaf issues in dependency order. Do not let any command
perform reconciliation directly; every command named here writes desired state or
invokes an explicit helper capability only.

| Issue range | Required output |
| --- | --- |
| #147-#151 | PHP runtime and Composer dependency behavior, including `php:install <version>` and `composer:install <version> --php <php-version>`. |
| #152-#156 | Daemon reconcile loop, resource-agnostic supervisor, Mailpit process definition, and runnable observed status. |
| #157-#161 | Postgres first, then MySQL, with explicit `db:create`, `db:drop`, and `db:list` commands for each. |
| #162-#165 | Redis, Mailpit env, RustFS S3 resource, and redaction tests. |

Non-negotiable decisions:

- Stacked diff branch is `rewrite/epic-3-runtime-daemon-resources` and its base
  is `rewrite/epic-2-store-host-install`.
- Epic 3 PRs do not target `main` directly.
- Composer depends on a managed PHP runtime and never falls back to system PHP.
- Mailpit remains explicit mail capture behavior, not generic HTTP service behavior.
- Postgres is implemented before extracting shared database mechanics for MySQL.
- Database command MVP is exactly `db:create`, `db:drop`, and `db:list` for Postgres and MySQL.
- Supervisor public API and tests must not mention concrete resource names.
- RustFS credentials may render into declared env values but must be redacted from status and logs.

## Suggested Package Ownership

- `internal/resources/php` owns PHP runtime desired state, install integration,
  shims, and runtime status.
- `internal/resources/composer` owns Composer tool desired state, PHP dependency
  checks, and Composer shim behavior.
- `internal/control` owns daemon reconcile orchestration.
- `internal/supervisor` owns process lifecycle primitives only.
- `internal/resources/mailpit` owns Mailpit process definitions, ports, env, and
  status mapping.
- `internal/resources/postgres` owns Postgres version-line behavior, data paths,
  process definitions, env, and database commands.
- `internal/resources/mysql` owns MySQL version-line behavior, initialization,
  socket/PID behavior, privileges, env, and database commands.
- `internal/resources/redis` owns Redis process flags, data/log paths, env, and
  status mapping.
- `internal/resources/rustfs` owns RustFS credentials, API/console ports, S3 env,
  and redacted status.

## Feature 3.1: PHP Runtime And Composer Tooling

**Goal:** Make managed PHP and Composer usable without system PHP assumptions.

### Implementation Sequence

1. Add PHP runtime desired-state model and controller.
2. Integrate PHP with the install planner and canonical runtime paths.
3. Add `php:install` request handling that writes desired state only.
4. Add observed status for missing, pending, ready, blocked, and failed runtime
   states.
5. Add Composer desired state with required PHP runtime version.
6. Add Composer controller dependency checks.
7. Add Composer install request handling that records desired state and blocked
   status when PHP is missing.
8. Add atomic, runtime-aware PHP and Composer shim exposure.
9. Extend status output for runtime/tool dependency failures.

### Acceptance Notes

- Keep the public colon-style command contract.
- Do not use system PHP as an implicit fallback.
- Composer is a tool depending on a runtime, not a sibling runtime.

## Feature 3.2: Daemon And Supervisor With Mailpit

**Goal:** Add the reconcile loop and first supervised runnable resource.

### Implementation Sequence

1. Add daemon reconcile loop over desired-state resource records.
2. Add signal handling and wake-up behavior after durable state changes.
3. Add supervisor process definition type.
4. Add supervisor lifecycle API: start, stop, check, readiness, and log path.
5. Add restart budget and deterministic observed failure reasons.
6. Add Mailpit resource desired state and controller.
7. Add Mailpit process definition, ports, logs, readiness, and env values.
8. Persist observed status for PID, port, log path, last error, and last
   reconcile time.

### Acceptance Notes

- Supervisor APIs cannot mention PHP, Laravel, Mailpit, databases, or RustFS.
- Mailpit is the first runnable resource because it is useful and smaller than a
  database.
- Real process integration should stay narrow; most tests should use fake
  processes.

## Feature 3.3: Stateful Database Resources

**Goal:** Add explicit stateful database resources without hiding differences.

### Implementation Sequence

1. Add Postgres version-line desired state.
2. Add Postgres install detection, data/log paths, process definition,
   readiness, env values, and status mapping.
3. Add explicit Postgres database commands for create, drop, and list.
4. Extract shared mechanics only after Postgres behavior is clear.
5. Add MySQL version-line desired state.
6. Add MySQL initialization, socket/PID behavior, privilege handling, process
   definition, readiness, env values, and status mapping.
7. Add explicit MySQL database commands for create, drop, and list.
8. Add status coverage for missing install, stopped, running, blocked, and
   failed states.

### Acceptance Notes

- Do not create a fake generic database abstraction before both resources prove
  the shared shape.
- Database create/drop commands must be explicit.
- Data directories live under canonical stateful data paths.

## Feature 3.4: Cache, Mail, And Object Storage Resources

**Goal:** Add Redis and RustFS while preserving explicit resource capabilities.

### Implementation Sequence

1. Add Redis version-line desired state.
2. Add Redis process flags, data/log paths, readiness, env values, and status.
3. Add RustFS version-line desired state.
4. Add RustFS credential model, API port, console port, data/log paths, process
   definition, readiness, and status.
5. Add S3 env values for RustFS without printing credentials in status.
6. Add cache, mail, and object-storage env value aggregation for Epic 4 link
   work.
7. Add tests that Mailpit remains explicit mail capture, not a generic HTTP
   service.

### Acceptance Notes

- Redis is a runnable stateful cache resource.
- RustFS is an S3 resource with API and console behavior.
- Status may show credential presence or target names, but not secret values.

## Critical Path

1. PHP runtime controller.
2. Composer dependency and shims.
3. Daemon reconcile loop.
4. Resource-agnostic supervisor.
5. Mailpit runnable resource.
6. Postgres resource.
7. MySQL resource.
8. Redis and RustFS resources.

## Review Checklist

- [ ] Commands only request state changes.
- [ ] Controllers own resource reconciliation.
- [ ] Supervisor APIs are resource-agnostic.
- [ ] Resource packages expose capabilities instead of fake sameness.
- [ ] Tests isolate `HOME` for pv state.
- [ ] Tests do not use `t.Parallel()` with `t.Setenv`.
- [ ] No real artifact download workflow runs by default.
- [ ] Status and logs do not print secrets.
