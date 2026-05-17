# Feature PRD: Project Contract And Init

## Epic

- Parent: [Epic 4 - Laravel Project Experience](../README.md)
- Architecture: [Epic 4 Architecture](../arch.md)

## Goal

**Problem:** Laravel projects need explicit, reviewable configuration before pv can manage runtimes, services, env values, setup commands, and gateway routing. Hidden `.env` inference caused prototype drift.

**Solution:** Add a versioned `pv.yml` schema, parser, Laravel detection, deterministic generator, `pv init`, and overwrite protection.

**Impact:** Projects declare their local infrastructure in a committed contract before link or daemon reconciliation occurs.

## User Personas

- Laravel developer.
- Maintainer.

## User Stories

- As a Laravel developer, I want `pv init` to generate a reviewable `pv.yml` so that project infrastructure is explicit.
- As a Laravel developer, I want existing contracts preserved unless I force overwrite so that local config is not lost.
- As a maintainer, I want unsupported project types to fail clearly so that the MVP stays Laravel-first.

## Requirements

### Functional Requirements

- Parse `pv.yml` with required top-level `version: 1`.
- Validate PHP version, service declarations, aliases, env maps, setup command strings, and unknown fields.
- Detect Laravel using explicit project markers.
- Generate deterministic Laravel `pv.yml`.
- Implement `pv init` with overwrite refusal and `--force` behavior.

### Non-Functional Requirements

- `pv init` does not install resources.
- `pv init` does not mutate `.env`.
- Service choices are not inferred from `.env`.
- Generated YAML is stable enough to review in Git.

## Acceptance Criteria

- [ ] Valid minimal and full Laravel contracts parse.
- [ ] Unsupported versions and unknown fields fail clearly.
- [ ] Laravel detection uses explicit markers.
- [ ] Generated `pv.yml` is deterministic.
- [ ] Existing `pv.yml` is not overwritten without force.

## Out Of Scope

- Link, env, setup execution.
- Gateway.
- Generic PHP/static project polish.
