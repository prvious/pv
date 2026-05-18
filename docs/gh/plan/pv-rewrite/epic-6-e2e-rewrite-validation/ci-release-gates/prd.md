# Feature PRD: CI And Release Gates

## Feature Name

CI And Release Gates.

## Epic

- Parent: [Epic 6 - E2E Rewrite Validation](../prd.md)
- Architecture: [Epic 6 Architecture](../arch.md)

## Goal

**Problem:** E2E tests only protect the release if maintainers know which CI jobs
are required, which checks are local-safe, and how to record evidence. Without a
gate, E2E can become an optional local habit instead of release criteria.

**Solution:** Extend `.github/workflows/tests.yml` with E2E jobs. Tier 0 remains
safe for local runs. Tier 1 and Tier 2 run in GitHub-hosted CI VMs and refuse
local execution. The workflow records release evidence.

**Impact:** Release readiness includes clear, repeatable E2E evidence while
preventing privileged checks from mutating developer laptops.

## User Personas

- Maintainer.
- Automation user.
- Implementation agent.

## User Stories

- As a maintainer, I want one workflow for normal and E2E tests so that release readiness is unambiguous.
- As an automation user, I want each job to fail non-zero so that CI can enforce it.
- As a maintainer, I want Tier 1 and Tier 2 to run on GitHub VMs but refuse local execution so that laptops stay safe.

## Requirements

### Functional Requirements

- Extend `.github/workflows/tests.yml` instead of creating a separate E2E workflow.
- Define Tier 0 hermetic E2E job.
- Define Tier 1 local-process CI job.
- Define Tier 2 privileged-host CI job.
- Make Tier 1 and Tier 2 refuse local execution when `CI` is not `true`.
- Document exact jobs, local commands, and expected outputs.
- Add evidence template with scenario, command, expected result, actual result,
  log path, and follow-up issue fields.
- Ensure gate exits non-zero on failed scenarios.

### Non-Functional Requirements

- The single workflow is scriptable and reviewable.
- Human status goes to stderr unless machine output is explicitly requested.
- Tier 2 prints host actions before running.

## Acceptance Criteria

- [ ] `.github/workflows/tests.yml` includes normal Go checks and E2E tier jobs.
- [ ] Tier 0 job is documented and required for release readiness.
- [ ] Tier 1 and Tier 2 jobs run in GitHub-hosted CI VMs.
- [ ] Tier 1 and Tier 2 refuse local execution.
- [ ] Evidence template exists.
- [ ] Release docs identify required local-safe and CI-only E2E tiers.

## Out Of Scope

- Running privileged checks on developer laptops.
- Creating new artifact publishing workflows.
- Post-MVP scenario gates.
