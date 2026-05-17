# GitHub Automation: Laravel-First Local Control Plane

This file makes the project plan publishable without mixing planning with
implementation. Review the hierarchy in `issues-checklist.md` before running
creation commands.

## Labels

Create labels once:

```bash
gh label create epic --color 5319e7 --description "Large business capability"
gh label create feature --color 1d76db --description "Deliverable feature"
gh label create user-story --color 0e8a16 --description "User-facing work item"
gh label create enabler --color fbca04 --description "Technical enabling work"
gh label create test --color d4c5f9 --description "Test or QA work"
gh label create priority-critical --color b60205 --description "P0 critical path"
gh label create priority-high --color d93f0b --description "P1 high priority"
gh label create priority-medium --color fbca04 --description "P2 medium priority"
gh label create value-high --color 0e8a16 --description "High value"
gh label create value-medium --color c2e0c6 --description "Medium value"
gh label create control-plane --color 0052cc --description "Control-plane architecture"
gh label create laravel --color f05138 --description "Laravel project workflow"
gh label create runtime --color 5319e7 --description "Runtime and toolchain"
gh label create gateway --color 006b75 --description "Gateway, DNS, TLS, pv open"
gh label create resource --color 5319e7 --description "Managed resource"
gh label create quality --color 0e8a16 --description "Testing, status, QA, scope"
```

If a label already exists, `gh` will return an error for that label. That is
safe; continue with the missing labels.

## Creation Order

1. Create milestone `pv rewrite MVP`.
2. Create five epic issues from `issues-checklist.md`.
3. Create feature issues and link each to its parent epic.
4. Create story/enabler issues and link each to its parent feature.
5. Create test issues in parallel with feature issues.
6. Comment on legacy issues #96-#113 that they are superseded by the new plan.
7. Update PR #114 if reused so it does not close #96.

For Epic 1, use the focused package first:

- `docs/gh/plan/pv-rewrite/epic-1-rewrite-foundation/project-plan.md`
- `docs/gh/plan/pv-rewrite/epic-1-rewrite-foundation/implementation-plan.md`
- `docs/gh/plan/pv-rewrite/epic-1-rewrite-foundation/issues-checklist.md`
- `docs/gh/plan/pv-rewrite/epic-1-rewrite-foundation/test-strategy.md`
- `docs/gh/plan/pv-rewrite/epic-1-rewrite-foundation/qa-plan.md`

For Epic 2, use:

- `docs/gh/plan/pv-rewrite/epic-2-store-host-install-infrastructure/project-plan.md`
- `docs/gh/plan/pv-rewrite/epic-2-store-host-install-infrastructure/implementation-plan.md`
- `docs/gh/plan/pv-rewrite/epic-2-store-host-install-infrastructure/issues-checklist.md`
- `docs/gh/plan/pv-rewrite/epic-2-store-host-install-infrastructure/test-strategy.md`
- `docs/gh/plan/pv-rewrite/epic-2-store-host-install-infrastructure/qa-plan.md`

For Epic 3, use:

- `docs/gh/plan/pv-rewrite/epic-3-runtime-daemon-resources/project-plan.md`
- `docs/gh/plan/pv-rewrite/epic-3-runtime-daemon-resources/implementation-plan.md`
- `docs/gh/plan/pv-rewrite/epic-3-runtime-daemon-resources/issues-checklist.md`
- `docs/gh/plan/pv-rewrite/epic-3-runtime-daemon-resources/test-strategy.md`
- `docs/gh/plan/pv-rewrite/epic-3-runtime-daemon-resources/qa-plan.md`

For Epic 4, use:

- `docs/gh/plan/pv-rewrite/epic-4-laravel-project-experience/project-plan.md`
- `docs/gh/plan/pv-rewrite/epic-4-laravel-project-experience/implementation-plan.md`
- `docs/gh/plan/pv-rewrite/epic-4-laravel-project-experience/issues-checklist.md`
- `docs/gh/plan/pv-rewrite/epic-4-laravel-project-experience/test-strategy.md`
- `docs/gh/plan/pv-rewrite/epic-4-laravel-project-experience/qa-plan.md`

For Epic 5, use:

- `docs/gh/plan/pv-rewrite/epic-5-status-quality-scope-control/project-plan.md`
- `docs/gh/plan/pv-rewrite/epic-5-status-quality-scope-control/implementation-plan.md`
- `docs/gh/plan/pv-rewrite/epic-5-status-quality-scope-control/issues-checklist.md`
- `docs/gh/plan/pv-rewrite/epic-5-status-quality-scope-control/test-strategy.md`
- `docs/gh/plan/pv-rewrite/epic-5-status-quality-scope-control/qa-plan.md`

## Legacy Superseded Comment

Use this comment on #96-#113 when the new issue hierarchy is published:

```markdown
Superseded by the structured rewrite project plan in:

- `docs/gh/plan/pv-rewrite/README.md`
- `docs/gh/plan/pv-rewrite/laravel-first-control-plane/project-plan.md`
- `docs/gh/plan/pv-rewrite/laravel-first-control-plane/issues-checklist.md`

This issue remains useful as legacy/reference material, but new implementation
work should use the Epic -> Feature -> Story/Enabler -> Test hierarchy.
```

## Safe Publishing Commands

Create one issue at a time while reviewing the generated body:

```bash
gh issue create \
  --title "Epic: Rewrite Foundation" \
  --label epic \
  --label priority-critical \
  --label value-high \
  --label control-plane \
  --body-file /tmp/pv-epic-rewrite-foundation.md
```

Do not bulk-create every issue until the epic and feature structure is reviewed.
Bulk issue creation is easy to automate, but noisy to unwind.

## Project Board Fields

Recommended fields:

- Priority: P0, P1, P2, P3
- Value: High, Medium, Low
- Work Type: Epic, Feature, Story, Enabler, Test
- Estimate: story points or t-shirt size
- Epic: parent epic reference
- Feature: parent feature reference
- Legacy Reference: old issue or PR number

Recommended columns:

1. Backlog
2. Ready
3. In Progress
4. In Review
5. Testing
6. Done
