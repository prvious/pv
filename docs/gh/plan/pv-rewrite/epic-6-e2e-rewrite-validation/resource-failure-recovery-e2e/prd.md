# Feature PRD: Resource Failure And Recovery E2E

## Feature Name

Resource Failure And Recovery E2E.

## Epic

- Parent: [Epic 6 - E2E Rewrite Validation](../prd.md)
- Architecture: [Epic 6 Architecture](../arch.md)

## Goal

**Problem:** The rewrite promises clear status, logs, failures, next actions, and
self-healing behavior, but those promises are only meaningful if failure and
recovery paths work through real command workflows.

**Solution:** Add black-box E2E scenarios for missing installs, blocked
dependencies, setup failure, process crash, gateway failure, and recovery after
corrective action.

**Impact:** Maintainers can prove the product is diagnosable and recoverable, not
just successful on happy paths.

## User Personas

- Laravel developer.
- Maintainer.
- Automation user.

## User Stories

- As a Laravel developer, I want missing install errors to include next actions so that I can recover.
- As a maintainer, I want process and gateway failures visible in status so that support issues can be diagnosed.
- As an automation user, I want recovery workflows to clear stale failures so that scripts can validate fixes.

## Requirements

### Functional Requirements

- Simulate missing runtime or resource install.
- Simulate blocked dependency.
- Simulate setup command failure.
- Simulate runnable process crash.
- Simulate gateway route, TLS, or DNS failure in hermetic mode.
- Validate status includes log path, last error, next action, and redaction.
- Apply corrective action and validate status recovery.

### Non-Functional Requirements

- Tier 0 uses fake processes and fake host adapters.
- Real process checks are Tier 1 and run only in GitHub-hosted CI VMs.
- Privileged host checks are Tier 2 and run only in GitHub-hosted CI VMs.

## Acceptance Criteria

- [ ] Missing install scenario produces actionable error and status.
- [ ] Setup failure scenario fails fast and records evidence.
- [ ] Process crash scenario records log path and next action.
- [ ] Gateway failure scenario does not mutate host state by default.
- [ ] Recovery scenario clears stale failure state.

## Out Of Scope

- Load or soak testing.
- Full cross-platform matrix.
- Real privileged host mutation by default.
