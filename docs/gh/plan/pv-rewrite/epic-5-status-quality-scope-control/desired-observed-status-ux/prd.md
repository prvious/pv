# Feature PRD: Desired And Observed Status UX

## Epic

- Parent: [Epic 5 - Status, Quality, And Scope Control](../README.md)
- Architecture: [Epic 5 Architecture](../arch.md)

## Goal

**Problem:** Users cannot recover from local environment failures unless status explains what was requested, what happened, where logs are, and what to do next. Decorative status that hides desired/observed differences is not enough.

**Solution:** Add aggregate and targeted status views based on provider data, normalized states, failure metadata, next actions, redaction, and stable rendering.

**Impact:** The MVP becomes debuggable and scriptable across projects, runtimes, tools, resources, daemon, supervisor, and gateway.

## User Personas

- Laravel developer.
- Automation user.
- Maintainer.

## User Stories

- As a Laravel developer, I want `pv status` to show desired and observed state so that I can understand drift.
- As a Laravel developer, I want failures, logs, and next actions so that I can recover quickly.
- As an automation user, I want stable output rules so that scripts do not depend on decorative formatting.
- As a Laravel developer, I want targeted status views so that I can inspect one project, runtime, resource, or gateway.

## Requirements

### Functional Requirements

- Add aggregate status model and provider shape.
- Normalize healthy, stopped, missing install, blocked, crashed, failed, partial, and unknown states.
- Include log path, last error, last reconcile time, and next action where available.
- Add aggregate `pv status`.
- Add targeted `project`, `runtime`, `resource`, and `gateway` status views.
- Redact secret-like values.

### Non-Functional Requirements

- Providers return data, not rendered UI.
- Human output is stable and scriptable.
- stdout stays pipeable only for intentionally designed output.
- Tests use fake providers and deterministic clocks.

## Acceptance Criteria

- [ ] Aggregate status includes desired and observed state.
- [ ] Failure states include logs and next actions when available.
- [ ] Targeted views reuse aggregate data.
- [ ] Secret values are redacted.
- [ ] Output behavior is documented and tested.

## Out Of Scope

- TUI status.
- Implicit machine-readable output.
- Real resource process setup for status tests.
