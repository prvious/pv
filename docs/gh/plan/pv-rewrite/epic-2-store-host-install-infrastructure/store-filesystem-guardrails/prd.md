# Feature PRD: Store And Filesystem Guardrails

## Epic

- Parent: [Epic 2 - Store, Host, And Install Infrastructure](../README.md)
- Architecture: [Epic 2 Architecture](../arch.md)

## Goal

**Problem:** Without explicit path and store rules, every resource package can invent its own binary, data, log, cache, and state layout. Future migrations also become ambiguous if machine-owned state has no schema version or migration record.

**Solution:** Add canonical host path helpers, layout validation, store schema versioning, applied migration records, and a documented `pv.yml` contract-version ownership decision.

**Impact:** Resource work in Epics 3 and 4 starts with one visible filesystem and migration model.

## User Personas

- Maintainer.
- Implementation agent.

## User Stories

- As a maintainer, I want canonical pv paths so that resources cannot drift into ambiguous locations.
- As a maintainer, I want visible store schema and migration seams so that state upgrades are deliberate.
- As an implementation agent, I want the contract-version owner documented so that Epic 4 does not rediscover the decision.

## Requirements

### Functional Requirements

- Add helpers for bin, runtimes, tools, services, data, logs, state, cache, and config.
- Validate resource names and versions as safe path segments.
- Add schema version to machine-owned state.
- Add applied migration record model and migration runner seam.
- Record the `pv.yml` version decision as top-level `version: 1`, implemented by Epic 4 issue #171.

### Non-Functional Requirements

- Tests touching pv state must isolate `HOME`.
- Layout validation must prevent real binaries under `~/.pv/bin`.
- Migration failures must be explicit.
- Checksum/integrity metadata is deferred and not part of Epic 2 implementation.

## Acceptance Criteria

- [ ] Path helpers cover all canonical families.
- [ ] Unsafe path segments are rejected.
- [ ] Store exposes schema version.
- [ ] Applied migrations can be recorded.
- [ ] Migration runner executes pending migrations in order.
- [ ] Contract-version decision is documented and linked to Epic 4.

## Out Of Scope

- Full project contract parser.
- Real resource installation.
- Daemon or supervisor behavior.
- Store migration checksums.
