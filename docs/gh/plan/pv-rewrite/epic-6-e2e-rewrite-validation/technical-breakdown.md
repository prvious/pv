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
| 1 | Local process | no | Real daemon/supervisor process checks in temp roots and allocated ports. |
| 2 | Privileged host | no | DNS, TLS trust, and browser behavior requiring host mutation or trust changes. |

Tier 0 is the MVP release gate. Tiers 1 and 2 require explicit flags, build tags,
or environment variables and must print the host actions they will perform before
running.

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
