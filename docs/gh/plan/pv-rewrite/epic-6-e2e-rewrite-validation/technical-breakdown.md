# Technical Breakdown: Epic 6 - E2E Rewrite Validation

## Module Roles

| Module | Responsibility |
| --- | --- |
| `internal/e2e` or `test/e2e` | Harness helpers, sandbox setup, command runner, assertions, cleanup. |
| `test/e2e/fixtures` | Minimal Laravel fixture files and fixture mutation helpers. |
| `test/e2e/fakes` | Fake artifact catalog, fake process binaries, fake host behavior. |
| `test/e2e/scenarios` | Black-box E2E scenario tests grouped by workflow. |
| `scripts` or documented `go test` command | Release gate entrypoint for required E2E tier. |

## E2E Tiers

| Tier | Name | Required by default | Scope |
| --- | --- | --- | --- |
| 0 | Hermetic | yes | Compiled binary, temp HOME, fake host adapters, fake artifacts, fake processes. |
| 1 | CI local process | yes in GitHub CI, no locally | Real daemon/supervisor process checks in temp roots and allocated ports. |
| 2 | CI privileged host | yes in GitHub CI, no locally | DNS, TLS trust, and browser behavior requiring host mutation or trust changes. |

Tier 0 is local-safe and runs in CI. Tiers 1 and 2 run in GitHub-hosted CI VMs
because those VMs are disposable. Tiers 1 and 2 must fail closed outside CI so
they do not mutate a developer laptop. Tier 2 must print the host actions it will
perform before running.

## CI Workflow Contract

Epic 6 extends `.github/workflows/tests.yml`; it does not add a separate
`.github/workflows/e2e.yml`.

Required jobs after Epic 6 lands:

| Job | Purpose |
| --- | --- |
| `go` | Format, vet, build, and unit/integration tests for the root rewrite module. |
| `e2e-tier0` | Hermetic E2E scenarios, safe locally and in CI. |
| `e2e-tier1` | Real daemon/supervisor process E2E in GitHub-hosted CI VM only. |
| `e2e-tier2` | DNS, TLS trust, and browser E2E in GitHub-hosted CI VM only. |

The legacy `.github/workflows/e2e.yml` must be removed or disabled when the Epic
6 jobs are added to `tests.yml`.

## Required Scenario Groups

| Scenario group | Required checks |
| --- | --- |
| Harness | Binary build, isolated HOME, temp project root, cleanup, deterministic ports, log capture. |
| Init | `pv init`, deterministic `pv.yml`, no `.env` mutation, overwrite refusal. |
| Link | `pv link`, durable project desired state, declared-only env writes, setup command execution, daemon signal ordering. |
| Status | Desired/observed status, next actions, log paths, redaction, targeted views. |
| Helpers | `pv artisan`, `pv db`, `pv mail`, `pv s3` route through current project and declared resources. |
| Failure | Missing install, blocked dependency, setup failure, crashed process, gateway route failure. |
| Recovery | Correct command or state change clears blocked/failed status in a follow-up run. |
| Release gate | Required Tier 0 command exits non-zero on failed scenario and writes evidence. |

## Sandbox Contract

Every E2E test must set:

- isolated `HOME`;
- isolated pv state root;
- isolated project root;
- isolated cache, config, logs, and data roots;
- deterministic test name prefix for files and processes;
- allocated ports from the harness;
- cleanup for files and processes created by the test.

No default E2E test may read or write the user's real `~/.pv`, `/etc/hosts`,
trust stores, keychains, browsers, or global package caches.

## Command Runner Contract

The runner invokes the compiled `pv` binary and records:

- argv;
- working directory;
- environment diff from the parent process;
- stdout;
- stderr;
- exit code;
- elapsed time;
- relevant log file paths.

Assertions must use public command behavior and filesystem outputs, not private
package internals.

## Non-Goals

- No Docker or VM dependency.
- No Playwright/browser automation in MVP E2E.
- No default network artifact downloads.
- No legacy prototype E2E.
- No post-MVP capability scenarios.
- No separate rewrite E2E workflow file.
