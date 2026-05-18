# Feature PRD: Prototype Isolation And Root Scaffold

## Epic

- Parent: [Epic 1 - Rewrite Foundation](../README.md)
- Architecture: [Epic 1 Architecture](../arch.md)

## Goal

**Problem:** The rewrite cannot start cleanly while active code and prototype code share the repository root. Contributors need an unambiguous workspace and a minimal CLI surface that is scriptable from day one.

**Solution:** Move the prototype into `legacy/prototype`, create a fresh root Go module, and add a minimal root CLI with help, version, and unknown-command behavior.

**Impact:** Future rewrite work starts from a clean module and avoids carrying prototype dependencies or command shape forward by accident.

## User Personas

- Maintainer.
- Automation user.
- Implementation agent.

## User Stories

- As a maintainer, I want the prototype isolated so that root rewrite code cannot import it accidentally.
- As an automation user, I want help, version, and usage errors to be stable so that scripts can interact with pv safely.
- As an implementation agent, I want root/prototype working rules documented so that I modify the correct code.

## Requirements

### Functional Requirements

- Move the prototype into `legacy/prototype` as a buildable module.
- Create fresh root `go.mod`, `main.go`, and `internal/cli`.
- Implement `help`, `version`, and unknown command handling.
- Keep stdout for pipeable output and stderr for human errors/status.
- Document active rewrite versus legacy prototype boundaries.

### Non-Functional Requirements

- Do not add Fang by default.
- Root and prototype verification commands must be copy-pasteable.
- New root code must not import `legacy/prototype` packages.

## Acceptance Criteria

- [ ] `legacy/prototype` builds independently.
- [ ] Root module builds independently.
- [ ] `help` and `version` behavior is tested.
- [ ] Unknown command behavior is tested.
- [ ] Active/reference boundaries are documented.
- [ ] No root dependency on prototype packages exists.

## Out Of Scope

- Desired-state store.
- Mago tracer.
- PHP, Composer, daemon, supervisor, Laravel contracts, gateway, or services.
