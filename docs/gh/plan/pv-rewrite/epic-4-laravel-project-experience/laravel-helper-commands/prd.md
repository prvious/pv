# Feature PRD: Laravel Helper Commands

## Epic

- Parent: [Epic 4 - Laravel Project Experience](../README.md)
- Architecture: [Epic 4 Architecture](../arch.md)

## Goal

**Problem:** Daily Laravel workflows need convenient commands, but helpers must not bypass the project contract or auto-create missing resources. They must route through managed PHP and declared resources.

**Solution:** Add current-project resolution and helper commands for Artisan, database, mail, and S3 workflows.

**Impact:** Developers get a coherent Laravel workflow without hidden resource setup or ambiguous target selection.

## User Personas

- Laravel developer.
- Automation user.

## User Stories

- As a Laravel developer, I want `pv artisan` to run through managed PHP so that Artisan uses the linked project's runtime.
- As a Laravel developer, I want `pv db` to target the declared database resource so that database actions are explicit.
- As a Laravel developer, I want `pv mail` to inspect the declared Mailpit resource so that local mail capture is easy.
- As a Laravel developer, I want `pv s3` to target the declared RustFS resource without printing secrets.

## Requirements

### Functional Requirements

- Resolve current linked project for every helper.
- Add `pv artisan` with argument passthrough to Artisan through managed PHP.
- Add `pv db` routing to declared Postgres or MySQL resource.
- Add `pv mail` routing to declared Mailpit resource.
- Add `pv s3` routing to declared RustFS resource.
- Return actionable errors for missing project, missing resource, or missing runtime.

### Non-Functional Requirements

- Helpers do not auto-create missing resources.
- Helpers keep stdout/stderr scriptable.
- S3 helper output must not print secret values.

## Acceptance Criteria

- [ ] Helpers resolve current project first.
- [ ] `pv artisan` uses managed PHP.
- [ ] `pv db` targets only declared database resources.
- [ ] `pv mail` targets declared Mailpit.
- [ ] `pv s3` targets declared RustFS and redacts secrets.

## Out Of Scope

- Generic command runner.
- Queue, scheduler, or worker supervision.
- Auto-creating resources from helpers.
