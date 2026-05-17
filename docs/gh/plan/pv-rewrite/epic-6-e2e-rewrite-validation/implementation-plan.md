# Implementation Plan: Epic 6 - E2E Rewrite Validation

## Execution Rules

- Treat Epic 6 as the final rewrite stack gate after Epic 5.
- E2E tests exercise the compiled `pv` binary through public command behavior.
- Default E2E runs are hermetic and must not mutate real host state.
- Real process and privileged host checks are opt-in.
- Use Go for repository logic and harness code.
- Before Go work, activate `golang-pro` and `modern-go`.
- Before each commit, run `go-simplifier` on changed Go code.
- Always add or update tests for changed behavior.
- Before handing off Go changes, run:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Implementation Contract

Execute the Epic 6 leaf issues in dependency order. Keep harness work separate
from scenario work until the binary runner and sandbox are tested.

| Issue | Task | Required output |
| --- | --- | --- |
| E6-EN1 | Task 1 | E2E can build or locate the active rewrite `pv` binary. |
| E6-EN2 | Task 2 | Sandbox and command runner isolate HOME, project root, ports, logs, and cleanup. |
| E6-EN3 | Task 3 | Minimal Laravel fixture generator creates deterministic files. |
| E6-T1 | Verification | Harness isolation and cleanup tests pass. |
| E6-S1 | Task 4 | `pv init` E2E validates deterministic `pv.yml` and no `.env` mutation. |
| E6-S2 | Task 5 | `pv link` E2E validates project state, env writes, setup, and signal ordering. |
| E6-S3 | Task 6 | Status and helper E2E validates public workflows. |
| E6-T2 | Verification | Laravel lifecycle E2E tests pass. |
| E6-S4 | Task 7 | Missing install and blocked dependency failures are tested. |
| E6-S5 | Task 8 | Setup, process, and gateway failures are tested. |
| E6-S6 | Task 9 | Recovery after corrective action is tested. |
| E6-T3 | Verification | Failure and recovery E2E tests pass. |
| E6-EN4 | Task 10 | Tier controls for hermetic, local-process, and privileged-host are implemented. |
| E6-EN5 | Task 11 | Tier 0 release gate command is documented and scriptable. |
| E6-S7 | Task 12 | E2E evidence template is produced for release. |
| E6-T4 | Verification | CI and release gate behavior tests pass. |

Non-negotiable decisions:

- Stacked diff branch is `rewrite/epic-6-e2e-rewrite-validation` and its base is
  `rewrite/epic-5-status-quality-scope`.
- Epic 6 PRs do not target `main` directly.
- Tier 0 E2E is required for MVP release readiness.
- Tier 0 E2E must not touch real `~/.pv`, `/etc/hosts`, trust stores, keychains,
  browsers, network artifact downloads, or live resources.
- Tiers 1 and 2 require explicit opt-in controls.

## Task 1: Build Or Locate Active pv Binary

**Files likely affected:**

- `test/e2e/harness/binary.go`
- `test/e2e/harness/binary_test.go`
- E2E documentation or script entrypoint

**Steps:**

1. Build the active rewrite binary from the repository root into a temp path.
2. Record binary path in the E2E harness.
3. Fail with a clear error if the binary cannot be built.
4. Do not use the legacy prototype binary.

**Acceptance criteria:**

- E2E tests invoke the active rewrite binary.
- Binary build failure includes command, working directory, and error output.

## Task 2: Add Sandbox And Command Runner

**Files likely affected:**

- `test/e2e/harness/sandbox.go`
- `test/e2e/harness/runner.go`
- `test/e2e/harness/*_test.go`

**Steps:**

1. Create isolated `HOME`, project root, pv state root, cache, config, data, and logs.
2. Allocate deterministic test ports through the harness.
3. Create command runner for argv, working directory, env, stdout, stderr, exit code, elapsed time, and logs.
4. Add cleanup for files and processes created by the test.
5. Reject attempts to use the real user home or real `~/.pv`.

**Acceptance criteria:**

- Tests prove sandbox paths are under temp directories.
- Tests prove command results capture stdout, stderr, and exit code.
- Tests prove cleanup removes sandbox-owned processes and files.

## Task 3: Add Laravel Fixture Generator

**Files likely affected:**

- `test/e2e/fixtures/laravel.go`
- `test/e2e/fixtures/laravel_test.go`

**Steps:**

1. Generate minimal Laravel markers expected by Epic 4 detection.
2. Generate deterministic `composer.json`, `.env.example`, and Artisan marker files.
3. Add fixture mutation helpers for existing `pv.yml`, broken setup command, and declared resources.
4. Keep fixture small and generated inside the sandbox.

**Acceptance criteria:**

- Fixture is deterministic across runs.
- Fixture is sufficient for `pv init` and `pv link` scenarios.

## Task 4: Validate `pv init` Lifecycle

**Steps:**

1. Run `pv init` in a fresh Laravel fixture.
2. Assert generated `pv.yml` includes `version: 1` and declared PHP.
3. Assert `.env` is not created or mutated by init.
4. Run init again and assert overwrite refusal.
5. Run forced init and assert deterministic output.

**Acceptance criteria:**

- `pv init` E2E proves contract generation and overwrite behavior.

## Task 5: Validate `pv link`, Env, Setup, And Signal Ordering

**Steps:**

1. Run `pv link` with declared env keys and setup commands.
2. Assert project desired state exists in the sandboxed store.
3. Assert `.env` contains only declared pv-managed env keys and preserves user lines.
4. Assert setup commands run from project root with managed PHP first on `PATH`.
5. Assert daemon signal happens after durable state write using the public evidence exposed by the rewrite.

**Acceptance criteria:**

- `pv link` E2E proves declared-only env writes and setup behavior.

## Task 6: Validate Status And Helper Workflows

**Steps:**

1. Run aggregate `pv status` after link.
2. Run targeted project, runtime, resource, and gateway status views.
3. Run `pv artisan` with argument passthrough.
4. Run `pv db`, `pv mail`, and `pv s3` against declared fake resources.
5. Assert stdout/stderr and exit codes are scriptable.

**Acceptance criteria:**

- Status and helpers work through public CLI behavior.
- Secret-like values are not printed.

## Task 7: Validate Missing Install And Blocked Dependency Failures

**Steps:**

1. Create a contract that declares a missing runtime or resource.
2. Run link or reconcile path that reaches blocked status.
3. Assert error includes next action.
4. Assert status records blocked or missing-install state.

**Acceptance criteria:**

- Missing prerequisites fail clearly and are visible in status.

## Task 8: Validate Setup, Process, And Gateway Failures

**Steps:**

1. Run setup with a failing command and assert fail-fast behavior.
2. Simulate a crashed runnable process with a fake process binary.
3. Simulate gateway route or TLS adapter failure in hermetic mode.
4. Assert logs, last error, and next action appear in status.

**Acceptance criteria:**

- E2E covers setup failure, process failure, and gateway failure.

## Task 9: Validate Recovery After Corrective Action

**Steps:**

1. Apply the next action from a missing install or blocked dependency failure.
2. Re-run reconcile or link workflow.
3. Assert status transitions from blocked/failed to healthy or pending expected state.
4. Assert stale error text no longer appears after recovery.

**Acceptance criteria:**

- Failure scenarios include follow-up recovery validation.

## Task 10: Define E2E Tiers And Opt-In Controls

**Steps:**

1. Define Tier 0 hermetic tests as the default release gate.
2. Define Tier 1 local-process tests behind explicit flag, build tag, or environment variable.
3. Define Tier 2 privileged-host tests behind explicit flag, build tag, or environment variable.
4. Print intended host actions before Tier 2 runs.

**Acceptance criteria:**

- Default command runs Tier 0 only.
- Tiers 1 and 2 cannot run accidentally.

## Task 11: Add Release Gate Command

**Steps:**

1. Document the exact Tier 0 command.
2. Ensure failure exits non-zero.
3. Ensure evidence output names scenario, command, expected result, actual result, and log path.
4. Keep human status on stderr and machine-readable output explicit.

**Acceptance criteria:**

- Release gate command is scriptable and documented.

## Task 12: Produce E2E Release Evidence Template

**Steps:**

1. Add a markdown evidence template for release validation.
2. Include Tier 0 command output location.
3. Include optional Tier 1 and Tier 2 sections.
4. Include follow-up issue field for failures.

**Acceptance criteria:**

- Maintainers can record E2E evidence without inventing a format.

## Verification

Run root verification for Go changes:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Run the required Tier 0 E2E release gate after it exists. Do not run Tier 1 or
Tier 2 without explicit user approval.
