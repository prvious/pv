# Feature PRD: Cache, Mail, And Object Storage Resources

## Epic

- Parent: [Epic 3 - Runtime, Daemon, And Resources](../README.md)
- Architecture: [Epic 3 Architecture](../arch.md)

## Goal

**Problem:** Laravel projects need cache, mail capture, and S3-compatible storage, but these resources expose different capabilities and secrets. Treating them as a generic HTTP service would hide important behavior.

**Solution:** Add Redis as a stateful cache resource, keep Mailpit explicit as mail capture, and add RustFS as an S3 resource with credential, API, console, route, env, and redaction behavior.

**Impact:** Epic 4 can render declared cache, mail, and object storage env values and helper commands can route to real resource capabilities.

## User Personas

- Laravel developer.
- Maintainer.

## User Stories

- As a Laravel developer, I want Redis managed by pv so that cache configuration is local and declared.
- As a Laravel developer, I want Mailpit env values so that local mail capture works through declared resources.
- As a Laravel developer, I want RustFS managed by pv so that S3-compatible local storage is available.
- As a maintainer, I want secrets redacted so that status and logs do not leak credentials.

## Requirements

### Functional Requirements

- Add Redis desired state, process flags, data/log paths, readiness, env values, and status.
- Keep Mailpit env and status behavior explicit as mail capture.
- Add RustFS desired state, credentials, API port, console port, data/log paths, process definition, readiness, env values, and status.
- Add cache, mail, and object-storage env provider values for Epic 4.

### Non-Functional Requirements

- RustFS secret values must not print in status or logs.
- Tests use fake processes and deterministic secret sentinel values.
- No unit test starts real Redis, Mailpit, or RustFS processes.

## Acceptance Criteria

- [ ] Redis status and env values are explicit.
- [ ] Mailpit remains mail capture, not generic HTTP service behavior.
- [ ] RustFS status includes route/port context without secret values.
- [ ] Cache, mail, and S3 env providers do not inspect `.env`.

## Out Of Scope

- Bucket migration tooling.
- Generic object storage provider abstraction.
- Queue or scheduler supervision.
