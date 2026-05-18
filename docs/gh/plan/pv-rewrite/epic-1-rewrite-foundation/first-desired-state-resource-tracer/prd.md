# Feature PRD: First Desired-State Resource Tracer

## Epic

- Parent: [Epic 1 - Rewrite Foundation](../README.md)
- Architecture: [Epic 1 Architecture](../arch.md)

## Goal

**Problem:** The rewrite architecture needs proof that commands can request state changes while controllers reconcile and report observed status separately. Without a small tracer, later resource work can drift back into command-driven orchestration.

**Solution:** Add a Mago install tracer with desired state, observed status, a controller, a fake installer seam, and status output.

**Impact:** The first vertical slice proves the control-plane rule before PHP, services, daemon, or Laravel workflows are introduced.

## User Personas

- Maintainer.
- Automation user.

## User Stories

- As a maintainer, I want a command to request one installable resource so that command behavior stays thin.
- As a maintainer, I want a controller to reconcile the requested resource so that work happens outside the command layer.
- As an automation user, I want status to show pending, ready, and failed states so that scripts can inspect the tracer.

## Requirements

### Functional Requirements

- Add desired resource state for Mago with requested version.
- Add observed status with state, last reconcile time, last error, and next action.
- Add `mago:install <version>` as the exact tracer command.
- Add a Mago controller that uses an installer adapter.
- Add `status` output for no desired state, pending, ready, and failed.

### Non-Functional Requirements

- Desired writes must not create observed status.
- Observed writes must not mutate desired state.
- Tests must use fake installers and deterministic clocks.
- No real artifact downloads are allowed in Epic 1 tests.

## Acceptance Criteria

- [ ] Command validates one version argument.
- [ ] Command writes desired state only.
- [ ] Controller no-ops without desired state.
- [ ] Controller writes ready observed status on fake install success.
- [ ] Controller writes failed observed status and next action on fake install failure.
- [ ] Status distinguishes desired and observed state.

## Out Of Scope

- Real Mago artifact download.
- SQLite store.
- Daemon wake-up or background reconciliation.
- Any resource other than Mago.
