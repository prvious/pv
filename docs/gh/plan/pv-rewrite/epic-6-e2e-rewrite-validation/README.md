# Epic 6: E2E Rewrite Validation

This package is the focused execution plan for Epic 6 of the pv rewrite.

Epic 6 adds end-to-end validation for the new rewrite. The earlier epics define
the product and implementation slices; this epic proves those slices work
together through black-box workflows before release.

## Documents

- `prd.md` - Epic 6 product requirements.
- `arch.md` - Epic 6 architecture specification.
- `technical-breakdown.md` - harness, fixture, environment, and gate contracts.
- `project-plan.md` - Epic 6 work hierarchy, dependencies, risks, and done rules.
- `implementation-plan.md` - non-optional implementation sequence.
- `issues-checklist.md` - GitHub issue body templates and publishing tracker.
- `test-strategy.md` - E2E test strategy using ISTQB and ISO 25010 framing.
- `test-issues-checklist.md` - concrete test coverage checklist.
- `qa-plan.md` - quality gates and manual QA expectations.
- `e2e-harness-fixtures/prd.md` - Feature 6.1 PRD.
- `laravel-project-lifecycle-e2e/prd.md` - Feature 6.2 PRD.
- `resource-failure-recovery-e2e/prd.md` - Feature 6.3 PRD.
- `ci-release-gates/prd.md` - Feature 6.4 PRD.

## Stacked Diff Position

Epic 6 branch: `rewrite/epic-6-e2e-rewrite-validation`.

Base branch: `rewrite/epic-5-status-quality-scope`.

No Epic 6 PR targets `main` directly.

## Legacy Reference

There is no legacy flat issue for this epic. It exists because the first rewrite
planning pass did not create an E2E validation epic for the new architecture.
