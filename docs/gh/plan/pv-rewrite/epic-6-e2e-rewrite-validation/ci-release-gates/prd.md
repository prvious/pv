# Feature PRD: CI And Release Gates

## Feature Name

CI And Release Gates.

## Epic

- Parent: [Epic 6 - E2E Rewrite Validation](../prd.md)
- Architecture: [Epic 6 Architecture](../arch.md)

## Goal

**Problem:** E2E tests only protect the release if maintainers know which command
is required, which checks are opt-in, and how to record evidence. Without a gate,
E2E can become an optional local habit instead of release criteria.

**Solution:** Define E2E tiers, add a required Tier 0 release gate command, guard
Tier 1 and Tier 2 behind explicit opt-ins, and provide an evidence template.

**Impact:** Release readiness includes clear, repeatable E2E evidence without
accidental host mutation.

## User Personas

- Maintainer.
- Automation user.
- Implementation agent.

## User Stories

- As a maintainer, I want one required Tier 0 command so that release readiness is unambiguous.
- As an automation user, I want the command to fail non-zero so that CI can enforce it.
- As a maintainer, I want opt-in tiers documented so that real host checks are deliberate.

## Requirements

### Functional Requirements

- Define Tier 0 hermetic E2E gate.
- Define Tier 1 local-process opt-in control.
- Define Tier 2 privileged-host opt-in control.
- Document exact commands and expected outputs.
- Add evidence template with scenario, command, expected result, actual result,
  log path, and follow-up issue fields.
- Ensure gate exits non-zero on failed scenarios.

### Non-Functional Requirements

- Default gate is scriptable.
- Human status goes to stderr unless machine output is explicitly requested.
- Tier 2 prints host actions before running.

## Acceptance Criteria

- [ ] Tier 0 command is documented and required for release readiness.
- [ ] Tier 0 command fails closed.
- [ ] Tier 1 and Tier 2 cannot run accidentally.
- [ ] Evidence template exists.
- [ ] Release docs identify required and optional E2E tiers.

## Out Of Scope

- Running privileged checks in default CI.
- Creating new artifact publishing workflows.
- Post-MVP scenario gates.
