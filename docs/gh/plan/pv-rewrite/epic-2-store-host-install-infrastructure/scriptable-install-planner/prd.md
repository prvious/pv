# Feature PRD: Scriptable Install Planner

## Epic

- Parent: [Epic 2 - Store, Host, And Install Infrastructure](../README.md)
- Architecture: [Epic 2 Architecture](../arch.md)

## Goal

**Problem:** Runtime, tool, and service installs will duplicate planning, download, ordering, shim, state, and signal behavior unless the shared mechanics exist first. Failed installs can otherwise expose broken shims or stale ready state.

**Solution:** Build a shared install planner with item identity, dependency graph validation, bounded downloads, dependency-ordered execution, atomic shim exposure, durable persistence, and signal ordering.

**Impact:** Epics 3 and 4 can add real resources without recreating install orchestration per package.

## User Personas

- Maintainer.
- Automation user.

## User Stories

- As a maintainer, I want install plans to validate dependencies before work starts so that failures are predictable.
- As an automation user, I want failed installs to avoid exposing broken shims so that scripts do not run partial tools.
- As a maintainer, I want daemon signaling only after durable persistence so that reconciliation sees consistent state.

## Requirements

### Functional Requirements

- Define plan item kinds: runtime, tool, service.
- Validate duplicate identities and missing dependencies.
- Produce deterministic topological order.
- Download through a bounded scheduler.
- Execute installs in dependency order through an adapter.
- Write shims atomically after successful install.
- Persist desired state after durable work.
- Signal reconciliation only after persistence succeeds.

### Non-Functional Requirements

- Tests must use fake resolvers, downloaders, installers, and signal adapters.
- No network downloads or artifact workflows run in Epic 2.
- Failed and dry-run plans must not signal reconciliation.

## Acceptance Criteria

- [ ] Invalid plans fail before work starts.
- [ ] Download parallelism is bounded.
- [ ] Failed prerequisites skip dependent work.
- [ ] Shim exposure is atomic and cleans temp files on failure.
- [ ] Successful plans persist state before signaling.
- [ ] Failed plans do not advertise completed work.

## Out Of Scope

- Real PHP, Composer, database, Redis, Mailpit, or RustFS installers.
- Artifact publishing workflows.
- Daemon implementation.
