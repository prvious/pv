# Implementation Plan: Epic 2 - Store, Host, And Install Infrastructure

## Scope

Epic 2 builds infrastructure, not product workflows:

1. Canonical host path helpers.
2. Store schema and migration seam.
3. Project contract versioning decision.
4. Layout validation.
5. Install plan model and dependency graph.
6. Bounded download scheduler.
7. Dependency ordered install executor.
8. Atomic shim exposure.
9. Durable state persistence and daemon signaling seam after installs.

Do not implement real PHP, Composer, Postgres, MySQL, Redis, Mailpit, RustFS, or
Laravel behavior in Epic 2. Use fake adapters.

## Required Skills For Implementation

Before changing Go code:

- Activate `golang-pro`.
- Activate `modern-go`.
- Before each commit, run `go-simplifier` on changed Go code.

## Task 1: Add Canonical Host Path Helpers

**Files likely affected:**

- `internal/host/paths.go`
- `internal/host/paths_test.go`
- existing active rewrite code that constructs `~/.pv` paths

**Steps:**

1. Add a host path package for `~/.pv`.
2. Add helpers for:
   - `bin`;
   - `runtimes/php/<version>`;
   - `tools/<name>/<version>`;
   - `services/<name>/<version>/bin`;
   - `data/<name>/<version>`;
   - `logs/<name>/<version>.log`;
   - `state/pv.db`;
   - `cache/artifacts`;
   - `config`.
3. Validate version and resource name path segments.
4. Replace active rewrite path construction with helpers.
5. Add tests with isolated `HOME`.

**Acceptance criteria:**

- Active rewrite code uses canonical helpers.
- `bin/` is for shims/symlinks only.
- Real binaries cannot be represented as ambiguous top-level paths.

## Task 2: Add Store Schema And Migration Seam

**Files likely affected:**

- `internal/control/store.go` or new `internal/store`
- `internal/control/store_test.go` or new store tests

**Steps:**

1. Add schema version to persisted machine state.
2. Add applied migration record model.
3. Add migration runner interface.
4. Add one no-op or initial migration to prove the shape.
5. Ensure migrations are forward-only.
6. Add checksum or integrity field if implementation cost is low now; otherwise
   document it as an explicit follow-up in the issue.

**Acceptance criteria:**

- Store exposes schema version.
- Applied migrations are recorded.
- Migration runner can execute pending migrations in order.
- Migration failure does not silently reinterpret state.

## Task 3: Decide Project Contract Version Path

**Files likely affected:**

- project contract docs or placeholder package
- Epic 2 issue notes

**Steps:**

1. Decide whether contract versioning lands now or is deferred to the project
   contract epic.
2. If now, add a minimal `contract_version` representation to relevant model.
3. If deferred, document:
   - where it will live;
   - what blocks it;
   - which issue owns it later.
4. Add a test or documentation assertion so the decision is visible.

**Acceptance criteria:**

- Contract versioning is not forgotten.
- The path is explicit and reviewable.

## Task 4: Prevent Ambiguous Storage Locations

**Files likely affected:**

- `internal/host/paths.go`
- `internal/host/paths_test.go`

**Steps:**

1. Add layout validation helpers.
2. Reject or make impossible:
   - real binaries at `~/.pv/bin`;
   - service-specific top-level directories outside `services`, `data`, `logs`;
   - unvalidated version path segments;
   - data paths under binary roots.
3. Add regression tests for invalid paths and unsafe segments.

**Acceptance criteria:**

- Tests pin allowed and disallowed path families.
- Future resource packages have one obvious path API.

## Task 5: Add Install Plan Model And Dependency Graph

**Files likely affected:**

- `internal/installer/plan.go`
- `internal/installer/plan_test.go`

**Steps:**

1. Define plan item types for runtime, tool, and service.
2. Add identity fields: resource kind, name, version.
3. Add dependencies between plan items.
4. Add validation for duplicate identities and missing dependencies.
5. Add topological ordering.

**Acceptance criteria:**

- Plans can include PHP, Composer, Mago, and service placeholders.
- Dependency order is deterministic.
- Invalid plans fail before work starts.

## Task 6: Add Bounded Download Scheduler

**Files likely affected:**

- `internal/installer/download.go`
- `internal/installer/download_test.go`

**Steps:**

1. Add downloader adapter interface.
2. Add bounded worker scheduler.
3. Preserve deterministic result ordering.
4. Cancel or stop scheduling on context cancellation.
5. Record per-item failures.

**Acceptance criteria:**

- Parallelism never exceeds the configured bound.
- Context cancellation is honored.
- Failed downloads are reported clearly.

## Task 7: Execute Dependency Ordered Installs

**Files likely affected:**

- `internal/installer/execute.go`
- `internal/installer/execute_test.go`

**Steps:**

1. Add installer adapter interface.
2. Execute plan items in dependency order.
3. Stop dependent installs when prerequisites fail.
4. Return structured results for ready, skipped, and failed items.
5. Keep real resource installs out of Epic 2.

**Acceptance criteria:**

- Install order follows dependency graph.
- Failures prevent dependent work.
- Results are suitable for status/next action later.

## Task 8: Expose Shims Atomically After Install

**Files likely affected:**

- `internal/installer/shim.go`
- `internal/installer/shim_test.go`

**Steps:**

1. Add atomic shim writer.
2. Write temp file in target directory.
3. Set executable permissions.
4. Rename into place.
5. Clean up temp files on failure.

**Acceptance criteria:**

- Existing shim is replaced atomically.
- Partial temp files are cleaned up.
- Shim content is not exposed before successful write.

## Task 9: Persist Desired State And Signal After Durable Install

**Files likely affected:**

- `internal/installer/planner.go`
- `internal/installer/planner_test.go`
- control/store signal adapter as needed

**Steps:**

1. Add completion phase that persists desired state after durable install work.
2. Add daemon signal adapter seam.
3. Signal only after persistence succeeds.
4. Do not signal for failed or dry-run plans.

**Acceptance criteria:**

- Successful durable installs persist desired state.
- Daemon signaling happens after persistence.
- Failed plans do not advertise completed work.

## Verification

Run:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Do not run expensive artifact workflows for Epic 2.
