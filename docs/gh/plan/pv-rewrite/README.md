# pv Rewrite Planning Package

This directory is the planning source for the Laravel-first control-plane rewrite.

The older GitHub issues #96-#113 and PRs #114-#115 are reference material only.
They captured useful scope, but they are intentionally superseded by this plan
because they were too flat for implementation. New work should be organized as:

```text
Epic -> Feature -> Story or Enabler -> Test -> Implementation Task
```

## Documents

- `arch.md` - rewrite-wide architecture specification.
- `laravel-first-control-plane.md` - feature PRD distilled from legacy issue #96.
- `laravel-first-control-plane/technical-breakdown.md` - module and flow breakdown.
- `laravel-first-control-plane/implementation-plan.md` - implementation sequence.
- `laravel-first-control-plane/project-plan.md` - project plan and work hierarchy.
- `laravel-first-control-plane/issues-checklist.md` - GitHub issue creation checklist and issue bodies.
- `laravel-first-control-plane/test-strategy.md` - test strategy.
- `laravel-first-control-plane/test-issues-checklist.md` - test issue checklist.
- `laravel-first-control-plane/qa-plan.md` - quality gates and QA process.
- `laravel-first-control-plane/github-automation.md` - labels, creation order, and publishing commands.
- `stacked-diff-plan.md` - branch stack and PR base rules for epic implementation.
- `epic-1-rewrite-foundation/` - focused execution package for Epic 1.
- `epic-2-store-host-install-infrastructure/` - focused execution package for Epic 2.
- `epic-3-runtime-daemon-resources/` - focused execution package for Epic 3.
- `epic-4-laravel-project-experience/` - focused execution package for Epic 4.
- `epic-5-status-quality-scope-control/` - focused execution package for Epic 5.
- `post-mvp-backlog.md` - deferred scope, deferral reasons, and reconsideration triggers.
- `mvp-scope-checklist.md` - MVP boundary checks for issue and PR review.
- `issue-label-audit.md` - audit evidence for #116-#205 labels and milestone.

## Stacked Diff Rule

Implementation uses stacked epic branches. Epic 1 targets `rewrite/base`; Epic 2
targets Epic 1; Epic 3 targets Epic 2; Epic 4 targets Epic 3; Epic 5 targets
Epic 4. No rewrite implementation PR targets `main` directly. See
`stacked-diff-plan.md` for branch names and PR body requirements.

Each `epic-*` package must include:

- `README.md` - package index and legacy references.
- `arch.md` - epic architecture specification.
- `technical-breakdown.md` - module, data, flow, and boundary details.
- `project-plan.md` - hierarchy, dependencies, estimates, risks, and done rules.
- `implementation-plan.md` - non-optional issue execution sequence.
- `issues-checklist.md` - published issue tracker and issue bodies.
- `test-strategy.md` - ISTQB/ISO-aligned strategy and test matrix.
- `test-issues-checklist.md` - concrete test issue coverage checklist.
- `qa-plan.md` - quality gates and manual review checklist.
- one feature directory per feature, each containing `prd.md`.

## Label Audit

The current GitHub label audit is recorded in `issue-label-audit.md`. Issues
#116-#205 are on milestone `pv rewrite MVP`; container issues use epic/feature
labels and leaf issues use `ready-for-agent` with enabler, user-story, or test
labels.

## Legacy Reference

- #96 - original rewrite PRD issue.
- #97-#113 - original flat implementation issues.
- #114 - reference PR for prototype move, root scaffold, and first tracer.
- #115 - reference PR for PHP and Composer tracer.

Do not treat #96-#113 as the new execution hierarchy. The issue bodies are
source material for the new issue checklist.
