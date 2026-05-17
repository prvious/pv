# Implementation Plan: Epic 1 - Rewrite Foundation

## Scope

Epic 1 is the smallest rewrite foundation:

1. Isolate the prototype.
2. Scaffold the active root module.
3. Add a minimal scriptable CLI.
4. Add a desired/observed store seam.
5. Add one installable resource tracer.
6. Add status reporting for that tracer.

Do not implement PHP, Composer, daemon, supervisor, Laravel contracts, gateway,
or managed backing services in Epic 1.

## Required Skills For Implementation

Before changing Go code:

- Activate `golang-pro`.
- Activate `modern-go`.
- Before each commit, run `go-simplifier` on changed Go code.

## Implementation Contract

Execute the published leaf issues in this order. Do not merge adjacent issues
unless the issue checklist is updated first.

| Issue | Task | Required output |
| --- | --- | --- |
| #119 | Task 1 | Prototype builds from `legacy/prototype`. |
| #120 | Task 2 | Fresh root module and `internal/cli` seam exist. |
| #121 | Task 2 | `help`, `version`, and unknown command behavior are tested. |
| #122 | Verification | Root/prototype build and CLI tests pass. |
| #123 | Task 4 | Desired and observed store seam exists. |
| #124 | Task 6 | Exact tracer command is `mago:install <version>`. |
| #125 | Task 5 | Mago controller reconciles through fake installer seam. |
| #126 | Task 7 | `status` reports no desired, pending, ready, and failed states. |
| #127 | Verification | Control-plane tracer tests pass without real downloads. |

Non-negotiable decisions:

- The first tracer resource is Mago.
- The first tracer command is `mago:install <version>`.
- Epic 1 does not introduce daemon, supervisor, SQLite, Laravel, PHP, Composer,
  gateway, databases, Redis, Mailpit, or RustFS.

## Task 1: Move Prototype Into `legacy/prototype`

**Files likely affected:**

- `legacy/prototype/**`
- root `go.mod`
- root `go.sum`
- root `main.go`
- old root `cmd/**`
- old root `internal/**`
- root docs that describe active code layout

**Steps:**

1. Create `legacy/prototype`.
2. Move the current prototype Go module as a complete module.
3. Keep prototype imports internally consistent.
4. Keep old prototype docs with the prototype if they describe old behavior.
5. Leave repository-level docs and agent instructions at the root.
6. Verify the prototype builds and tests from `legacy/prototype`.

**Acceptance criteria:**

- `legacy/prototype` contains the old buildable Go module.
- Root no longer contains old command or implementation packages.
- New root code cannot import prototype packages accidentally.

## Task 2: Scaffold Active Root Module

**Files likely affected:**

- `go.mod`
- `go.sum`
- `main.go`
- `internal/cli/root.go`
- `internal/cli/root_test.go`
- `docs/gh/plan/pv-rewrite/**`
- rewrite working docs

**Steps:**

1. Create fresh root `go.mod`.
2. Add root `main.go`.
3. Add `internal/cli` with an explicit `Run(args, stdout, stderr)` style seam.
4. Implement `help`, `version`, and unknown command behavior.
5. Keep stdout reserved for pipeable output.
6. Write human status and usage errors to stderr.
7. Do not add Fang by default.

**Acceptance criteria:**

- Root module builds independently.
- `pv help` and `pv version` work.
- Unknown commands return an error and usage hint.
- Tests prove stdout/stderr behavior.

## Task 3: Document Rewrite Working Rules

**Files likely affected:**

- `docs/gh/plan/pv-rewrite/README.md`
- `docs/gh/plan/pv-rewrite/epic-1-rewrite-foundation/**`
- `CONTEXT.md`

**Steps:**

1. Document root as active rewrite workspace.
2. Document `legacy/prototype` as reference-only.
3. Document root verification commands.
4. Document prototype verification commands.
5. Document that new rewrite code must not import prototype packages.

**Acceptance criteria:**

- An agent can identify active code versus reference code without asking.
- Verification commands are copy-pasteable.

## Task 4: Add Desired And Observed Store Seam

**Files likely affected:**

- `internal/control/store.go`
- `internal/control/store_test.go`

**Steps:**

1. Define desired resource state with resource name and requested version.
2. Define observed status with resource name, desired version, state, last
   reconcile time, last error, and next action.
3. Add a small store interface for desired and observed operations.
4. Add a scaffold implementation that can be replaced by SQLite in Epic 2.
5. Persist desired and observed state separately.
6. Validate unsafe version strings.

**Acceptance criteria:**

- Desired writes do not create observed status.
- Observed writes do not mutate desired state.
- Invalid versions are rejected.
- Corrupted state returns a clear error.

## Task 5: Add First Installable Resource Controller

**Preferred tracer:** Mago.

**Files likely affected:**

- `internal/resources/mago/controller.go`
- `internal/resources/mago/controller_test.go`

**Steps:**

1. Add a controller that reads desired state for the resource.
2. No-op when desired state is absent.
3. Use an installer interface so tests can use fakes.
4. Record `ready` observed status after successful install.
5. Record `failed` observed status and next action after install failure.
6. Use deterministic clock injection in tests.

**Acceptance criteria:**

- Controller success path writes observed status.
- Controller failure path writes observed status and returns the cause.
- Controller tests do not download real artifacts.

## Task 6: Add Command For Desired Install Request

**Files likely affected:**

- `internal/cli/root.go`
- `internal/cli/root_test.go`

**Steps:**

1. Add `mago:install <version>` as the first tracer command.
2. Validate argument count and version.
3. Write desired state only.
4. Do not install directly from the command.
5. Print concise human status to stderr.

**Acceptance criteria:**

- Command writes desired state.
- Command does not write observed status.
- Usage errors are returned for missing or invalid args.

## Task 7: Add Status For Desired And Observed State

**Files likely affected:**

- `internal/cli/root.go`
- `internal/cli/root_test.go`

**Steps:**

1. Add `status` command for the first tracer.
2. Print desired state when present.
3. Print pending reconcile when observed status is missing.
4. Print ready/failed observed status when present.
5. Include next action on failure.

**Acceptance criteria:**

- Status can distinguish desired from observed.
- Status can show pending, ready, and failed states.
- Output remains stable enough for tests without overfitting spacing.

## Verification

Run root verification:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Run prototype verification if prototype files moved or changed:

```bash
cd legacy/prototype
gofmt -w .
go vet ./...
go build ./...
go test ./...
```
