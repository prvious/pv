# Feature PRD: Laravel Project Lifecycle E2E

## Feature Name

Laravel Project Lifecycle E2E.

## Epic

- Parent: [Epic 6 - E2E Rewrite Validation](../prd.md)
- Architecture: [Epic 6 Architecture](../arch.md)

## Goal

**Problem:** Unit and integration tests can pass while the real Laravel developer
workflow fails across command boundaries, file writes, setup execution, status,
and helper routing.

**Solution:** Add black-box E2E scenarios for `pv init`, `pv link`, status, and
helper commands using the compiled pv binary and sandboxed Laravel fixtures.

**Impact:** The MVP workflow is validated the way a Laravel developer uses it.

## User Personas

- Laravel developer.
- Maintainer.
- Automation user.

## User Stories

- As a Laravel developer, I want `pv init` E2E coverage so that generated contracts work in real projects.
- As a Laravel developer, I want `pv link` E2E coverage so that env writes and setup commands are safe.
- As a Laravel developer, I want status and helper E2E coverage so that daily workflows route correctly.

## Requirements

### Functional Requirements

- Run `pv init` in a fresh Laravel fixture.
- Validate deterministic `pv.yml` with `version: 1`.
- Validate init overwrite refusal and forced overwrite.
- Run `pv link` from a declared project contract.
- Validate declared-only `.env` writes and setup execution.
- Validate aggregate and targeted status after link.
- Validate `pv artisan`, `pv db`, `pv mail`, and `pv s3` through current project state.

### Non-Functional Requirements

- Tests use the harness command runner.
- Tests assert public CLI behavior and filesystem/log outputs.
- Secret-like values are redacted.

## Acceptance Criteria

- [ ] Init lifecycle E2E passes.
- [ ] Link env/setup lifecycle E2E passes.
- [ ] Status views E2E passes.
- [ ] Helper routing E2E passes.
- [ ] No test infers services or env values from `.env`.

## Out Of Scope

- Browser automation.
- Real host DNS/TLS mutation.
- Post-MVP Laravel capabilities.
