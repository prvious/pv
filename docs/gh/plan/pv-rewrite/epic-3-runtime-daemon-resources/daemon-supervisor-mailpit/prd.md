# Feature PRD: Daemon And Supervisor With Mailpit

## Epic

- Parent: [Epic 3 - Runtime, Daemon, And Resources](../README.md)
- Architecture: [Epic 3 Architecture](../arch.md)

## Goal

**Problem:** Resource controllers need a long-running reconciler and process runner, but the process runner must not become a service-specific manager. A smaller runnable resource is needed before databases are added.

**Solution:** Add the daemon reconcile loop, resource-agnostic supervisor, and Mailpit as the first supervised runnable resource.

**Impact:** Later services can share lifecycle primitives while keeping their resource-specific behavior in resource packages.

## User Personas

- Laravel developer.
- Maintainer.

## User Stories

- As a maintainer, I want durable desired-state changes to wake reconciliation so that observed status catches up.
- As a maintainer, I want the supervisor to manage processes without resource names so that lifecycle logic stays reusable.
- As a Laravel developer, I want Mailpit managed by pv so that declared local mail capture is available.

## Requirements

### Functional Requirements

- Add daemon reconcile loop over desired resource records.
- Add signal/wake behavior after durable state changes.
- Add supervisor process definition, start, stop, check, readiness, log path, and restart budget.
- Add Mailpit desired state, process definition, ports, readiness, env values, and status.
- Persist PID, port, log path, last error, and last reconcile time for runnable resources.

### Non-Functional Requirements

- Supervisor public API and tests must not mention concrete resources.
- Unit tests use fake processes.
- Real process tests are narrow and opt-in.

## Acceptance Criteria

- [ ] Daemon dispatches controllers from desired state.
- [ ] Durable state changes can wake reconciliation.
- [ ] Supervisor remains resource-agnostic.
- [ ] Mailpit exposes SMTP and web ports.
- [ ] Runnable observed status includes process metadata and failure information.

## Out Of Scope

- Database resources.
- Gateway routing.
- Laravel project linking.
