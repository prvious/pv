# Implementation Plan: Laravel-First Local Control Plane

## Execution Rules

- Treat legacy issues #96-#113 and PRs #114-#115 as reference only.
- Execute the new hierarchy in this document and `project-plan.md`.
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

If prototype files change, also run the equivalent commands from
`legacy/prototype`.

## Epic 1: Rewrite Foundation

**Goal:** Establish the active root rewrite workspace and the first control-plane
vertical slice.

### Feature 1.1: Prototype Isolation And Root Scaffold

**Stories and enablers:**

- Move prototype into `legacy/prototype` as a buildable reference module.
- Scaffold the root module with minimal scriptable CLI behavior.
- Document root versus prototype working rules.
- Keep Fang and other presentation dependencies out by default.

**Implementation tasks:**

1. Move prototype files as a complete Go module.
2. Add root `go.mod`, `main.go`, and minimal CLI entrypoint.
3. Add help/version/unknown-command behavior.
4. Add root/prototype verification docs.
5. Add import-boundary review to PR checklist.

### Feature 1.2: First Desired-State Resource Tracer

**Stories and enablers:**

- Record desired state for one small installable resource.
- Reconcile desired state through a controller.
- Persist observed status separately.
- Report desired and observed status through CLI.

**Implementation tasks:**

1. Add desired/observed store interface.
2. Add scaffold store implementation with atomic writes.
3. Add first installable resource controller, using Mago as the tracer.
4. Add command that writes desired state only.
5. Add status command that can show pending observed state.
6. Add command, store, controller, and status tests.

## Epic 2: Store, Host, And Install Infrastructure

**Goal:** Prevent architectural drift before the resource surface grows.

### Feature 2.1: Store And Filesystem Guardrails

**Stories and enablers:**

- Canonicalize `~/.pv` layout.
- Add store schema and migration seams.
- Represent or explicitly defer `pv.yml` contract versioning.
- Prevent ambiguous top-level binary and data paths.

**Implementation tasks:**

1. Add `internal/host` path helpers for bin, runtimes, tools, services, data,
   logs, state, cache, and config.
2. Replace direct active-rewrite path construction with helpers.
3. Add schema version and applied migration records.
4. Add migration runner interface even if the first migration is empty.
5. Add layout validation tests.

### Feature 2.2: Scriptable Install Planner

**Stories and enablers:**

- Resolve install plans across runtimes, tools, and services.
- Download artifacts with bounded parallelism.
- Install in dependency order.
- Expose shims atomically.
- Persist desired state and signal reconciliation after durable changes.

**Implementation tasks:**

1. Define plan item model and dependency graph.
2. Add artifact resolver and fake downloader adapter.
3. Add bounded worker scheduling.
4. Add dependency-ordered installer execution.
5. Add atomic shim exposure helper.
6. Add failure handling that avoids advertising incomplete work.

## Epic 3: Runtime, Tools, Daemon, And Resources

**Goal:** Build the managed local infrastructure resources behind Laravel apps.

### Feature 3.1: PHP Runtime And Composer Tooling

**Stories and enablers:**

- Install and reconcile PHP runtimes.
- Install and reconcile Composer as a tool depending on a PHP runtime.
- Expose CLI shims atomically.
- Report blocked Composer status when PHP is missing.

**Implementation tasks:**

1. Add PHP runtime desired state and controller.
2. Add Composer desired state with required runtime version.
3. Add CLI commands for PHP and Composer install requests.
4. Add atomic Composer shim behavior.
5. Extend status output.

### Feature 3.2: Daemon And Supervisor With Mailpit

**Stories and enablers:**

- Add daemon reconcile loop.
- Add resource-agnostic supervisor.
- Add Mailpit as the first runnable resource.
- Record process metadata, logs, crashes, and next actions.

**Implementation tasks:**

1. Add supervisor process definitions and lifecycle API.
2. Add readiness probes and restart budget.
3. Add daemon reconcile loop and signal handling.
4. Add Mailpit controller and process definition.
5. Add observed status fields for PID, port, log path, failure, and last
   reconcile time.

### Feature 3.3: Stateful Database Resources

**Stories and enablers:**

- Add Postgres first.
- Add MySQL using shared mechanics only where justified.
- Keep database create/drop explicit.
- Expose env values through project contract variables.

**Implementation tasks:**

1. Add Postgres version-line state, install detection, process definition,
   readiness, env values, and explicit database commands.
2. Extract only earned shared mechanics.
3. Add MySQL version-line state, initialization, socket/PID behavior,
   privileges, process definition, env values, and explicit database commands.
4. Add status coverage for missing install, stopped, running, and failed states.

### Feature 3.4: Cache, Mail, And Object Storage Resources

**Stories and enablers:**

- Add Redis as runnable stateful cache resource.
- Add RustFS/S3 with credentials, API/console ports, routes, and env values.
- Keep Mailpit behavior explicit as mail capture, not a generic HTTP service.

**Implementation tasks:**

1. Add Redis version-line state, process flags, data/log paths, and env values.
2. Add RustFS version-line state, credential model, process definition, and S3
   env values.
3. Add RustFS route/status data without leaking credentials.
4. Ensure resource packages keep service-specific behavior explicit.

## Epic 4: Laravel Project Experience

**Goal:** Make the Laravel path first-class and explicit.

### Feature 4.1: Project Contract And Init

**Stories and enablers:**

- Parse and validate `pv.yml`.
- Generate reviewable Laravel contracts.
- Detect Laravel projects.
- Refuse overwrite unless forced.

**Implementation tasks:**

1. Add project contract schema.
2. Add template variable model.
3. Add Laravel detection.
4. Add Laravel contract generator.
5. Add `pv init` command.
6. Add unsupported/fallback project behavior.

### Feature 4.2: Link, Env, And Setup

**Stories and enablers:**

- Resolve project contract during link.
- Record durable desired project state.
- Render only declared env keys.
- Label pv-managed env writes.
- Run declared setup commands with pinned PHP runtime.
- Fail fast on setup errors and missing installs.

**Implementation tasks:**

1. Add project registry or project desired-state model.
2. Add env parser/merge writer with backups and labels.
3. Add setup runner with working directory, env propagation, PATH pinning, and
   stdout/stderr streaming.
4. Add `pv link` behavior that writes desired state and signals daemon.
5. Add clear errors for missing declared services.

### Feature 4.3: Gateway And `pv open`

**Stories and enablers:**

- Serve linked apps at HTTPS `.test` hosts.
- Support aliases in route and certificate behavior.
- Treat FrankenPHP as gateway infrastructure.
- Add `pv open`.

**Implementation tasks:**

1. Add gateway desired and observed state.
2. Add deterministic Caddy/FrankenPHP route rendering.
3. Add TLS certificate material and SAN behavior.
4. Add DNS adapter through host primitives.
5. Add gateway process definition for supervisor.
6. Add browser-open adapter and `pv open`.

### Feature 4.4: Laravel Helper Commands

**Stories and enablers:**

- Run Artisan through pinned PHP.
- Route database helpers to declared database resource.
- Route mail helpers to declared Mailpit resource.
- Route S3 helpers to declared RustFS resource.

**Implementation tasks:**

1. Add current-project resolution for helper commands.
2. Add `pv artisan`.
3. Add `pv db`.
4. Add `pv mail`.
5. Add `pv s3`.
6. Add missing-resource errors.

## Epic 5: Status, Quality, And Scope Control

**Goal:** Make the product understandable and keep MVP scope clean.

### Feature 5.1: Desired And Observed Status UX

**Stories and enablers:**

- Show desired state, observed state, failures, logs, and next actions.
- Support healthy, stopped, missing install, blocked, crashed, and partially
  reconciled states.
- Keep output scriptable.

**Implementation tasks:**

1. Add aggregate status model.
2. Add targeted status views for project, runtime, resource, and gateway.
3. Add stable human output to stderr.
4. Add pipeable output only when explicitly designed.
5. Add status tests across resource families.

### Feature 5.2: Post-MVP Backlog

**Stories and enablers:**

- Track intentionally omitted capabilities.
- Record why each is deferred.
- Record reconsideration triggers.
- Keep omitted work out of MVP issues.

**Implementation tasks:**

1. Create post-MVP backlog doc.
2. Populate from PRD out-of-scope section.
3. Add MVP scope checklist to PR template or planning docs.
4. Review new feature requests against the backlog before expanding MVP.
