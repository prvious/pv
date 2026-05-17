# Feature PRD: Link, Env, And Setup

## Epic

- Parent: [Epic 4 - Laravel Project Experience](../README.md)
- Architecture: [Epic 4 Architecture](../arch.md)

## Goal

**Problem:** Linking must turn an explicit project contract into durable desired state without clobbering user `.env` values or running hidden setup. Setup must use managed PHP, not whatever PHP is on the host path.

**Solution:** Add project desired state, managed env merge writer, setup runner, `pv link`, durable store-before-signal ordering, and actionable failure behavior.

**Impact:** Laravel projects become managed by pv through a reviewable, recoverable workflow.

## User Personas

- Laravel developer.
- Automation user.
- Maintainer.

## User Stories

- As a Laravel developer, I want `pv link` to apply my `pv.yml` so that pv manages the project explicitly.
- As a Laravel developer, I want pv-managed `.env` writes labeled and backed up so that my local values are preserved.
- As an automation user, I want setup to fail fast so that scripts stop on the first broken step.

## Requirements

### Functional Requirements

- Validate `pv.yml` before writing project desired state.
- Store project path, host, aliases, version, PHP, service declarations, env declarations, and setup commands.
- Render only declared env keys from project/resource providers.
- Back up `.env` before mutation.
- Update only pv-managed entries.
- Run ordered setup shell command strings from project root with managed PHP first on PATH.
- Signal daemon after durable state writes.
- Return actionable errors for missing declared resources, missing installs, and setup failures.

### Non-Functional Requirements

- Never infer services or env values from existing `.env`.
- Preserve user-authored `.env` lines.
- Stream setup stdout/stderr predictably.
- Tests isolate `HOME` for state.

## Acceptance Criteria

- [ ] `pv link` records durable project desired state.
- [ ] Store write happens before daemon signal.
- [ ] `.env` writes are declared-only, labeled, and backed up.
- [ ] Setup runs only declared commands.
- [ ] Setup uses managed PHP before system PHP.
- [ ] Missing resources and setup errors are actionable.

## Out Of Scope

- Gateway routing.
- Helper commands.
- Auto-creating missing resources.
