# Feature PRD: E2E Harness And Fixtures

## Feature Name

E2E Harness And Fixtures.

## Epic

- Parent: [Epic 6 - E2E Rewrite Validation](../prd.md)
- Architecture: [Epic 6 Architecture](../arch.md)

## Goal

**Problem:** End-to-end tests cannot be trusted if every scenario invents its own
binary build, temp directories, ports, output capture, fixtures, and cleanup.
Unsafe harness behavior can mutate the developer machine or accidentally invoke
the legacy prototype.

**Solution:** Build one E2E harness that compiles the active rewrite binary,
creates a sandbox, invokes pv through a command runner, captures evidence, and
generates deterministic Laravel fixtures.

**Impact:** Every later E2E scenario starts from safe, repeatable, black-box test
infrastructure.

## User Personas

- Maintainer.
- Implementation agent.
- Automation user.

## User Stories

- As a maintainer, I want the harness to invoke the active rewrite binary so that E2E tests validate the product being released.
- As an implementation agent, I want a sandboxed HOME and project root so that tests cannot mutate local user state.
- As an automation user, I want captured stdout, stderr, exit codes, and logs so that failures are diagnosable.

## Requirements

### Functional Requirements

- Build or locate active rewrite `pv` binary from repository root.
- Reject legacy prototype binary usage.
- Create isolated HOME, pv state root, cache, config, data, logs, and project root.
- Allocate ports through the harness.
- Capture argv, working directory, environment diff, stdout, stderr, exit code, elapsed time, and log paths.
- Generate deterministic minimal Laravel fixtures.
- Clean up sandbox-owned files and processes.

### Non-Functional Requirements

- Harness code uses Go.
- Tests that set `HOME` do not use `t.Parallel()`.
- Default harness behavior does not touch real host state.

## Acceptance Criteria

- [ ] Harness builds active rewrite binary into temp path.
- [ ] Sandbox paths are all under temp directories.
- [ ] Command runner captures stdout, stderr, and exit code.
- [ ] Laravel fixture is deterministic.
- [ ] Cleanup removes sandbox-owned processes and files.

## Out Of Scope

- Real DNS/TLS/browser mutation.
- Artifact downloads.
- Legacy prototype E2E.
