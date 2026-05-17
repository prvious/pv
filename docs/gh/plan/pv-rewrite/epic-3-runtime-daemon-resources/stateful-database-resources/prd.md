# Feature PRD: Stateful Database Resources

## Epic

- Parent: [Epic 3 - Runtime, Daemon, And Resources](../README.md)
- Architecture: [Epic 3 Architecture](../arch.md)

## Goal

**Problem:** Laravel projects commonly need Postgres or MySQL, but the two databases have different initialization, process, socket, privilege, and command semantics. A generic database abstraction introduced too early would hide those differences.

**Solution:** Implement Postgres first, extract only proven shared mechanics, then implement MySQL explicitly with its own initialization and process behavior.

**Impact:** Laravel env rendering and helper commands get reliable database capabilities without losing database-specific correctness.

## User Personas

- Laravel developer.
- Maintainer.

## User Stories

- As a Laravel developer, I want pv to manage a declared Postgres version line so that local database behavior is predictable.
- As a Laravel developer, I want pv to manage a declared MySQL version line so that MySQL projects do not require external setup.
- As a Laravel developer, I want explicit database create/drop/list commands so that setup remains reviewable.
- As a maintainer, I want MySQL-specific behavior preserved so that abstractions do not erase important differences.

## Requirements

### Functional Requirements

- Add Postgres version-line desired state, install detection, data/log paths, process definition, readiness, env values, status, and `db:create/drop/list` commands.
- Add MySQL version-line desired state, initialization, socket/PID behavior, privilege handling, process definition, readiness, env values, status, and `db:create/drop/list` commands.
- Expose env provider values without inspecting project `.env`.
- Cover missing install, stopped, running, blocked, and failed states.

### Non-Functional Requirements

- Data and logs use canonical host paths.
- Tests use fake processes by default.
- Shared mechanics are extracted only after both resources prove the same shape.

## Acceptance Criteria

- [ ] Postgres version-line desired state reconciles and reports status.
- [ ] Postgres database commands are explicit.
- [ ] MySQL version-line desired state reconciles and reports status.
- [ ] MySQL initialization, socket/PID, and privilege behavior are tested.
- [ ] Env values come from declared resource state only.

## Out Of Scope

- Dump/import tooling.
- Cross-line data upgrades.
- MariaDB.
