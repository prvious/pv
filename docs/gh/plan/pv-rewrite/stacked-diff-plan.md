# Stacked Diff Plan

The rewrite MVP must be implemented as a stack of epic branches. Do not target
`main` from individual epic, feature, story, enabler, or test PRs.

## Branch Stack

| Stack level | Branch | Base branch |
| --- | --- | --- |
| Rewrite base | `rewrite/base` | repository default branch only when the full rewrite stack is ready to integrate |
| Epic 1 | `rewrite/epic-1-foundation` | `rewrite/base` |
| Epic 2 | `rewrite/epic-2-store-host-install` | `rewrite/epic-1-foundation` |
| Epic 3 | `rewrite/epic-3-runtime-daemon-resources` | `rewrite/epic-2-store-host-install` |
| Epic 4 | `rewrite/epic-4-laravel-project-experience` | `rewrite/epic-3-runtime-daemon-resources` |
| Epic 5 | `rewrite/epic-5-status-quality-scope` | `rewrite/epic-4-laravel-project-experience` |

## Rules

- Every epic PR targets the previous epic branch in the table.
- Epic 1 targets `rewrite/base`, not `main`.
- Feature, story, enabler, and test PRs inside an epic target that epic's stack branch unless a smaller sub-stack is explicitly created under that epic branch.
- No implementation PR targets `main` directly.
- Do not rebase an epic branch onto `main` while the rewrite stack is in progress.
- If an earlier epic changes after later epic work starts, rebase or merge the later epic branch onto the updated previous epic branch.
- Keep PR descriptions explicit about the base branch and the stack position.
- Merge or land the stack from Epic 1 through Epic 5 in order.

## PR Body Requirement

Every implementation PR in this rewrite stack must include:

```markdown
Stack position: Epic N of 5
Base branch: rewrite/<previous-epic-branch>
Targets main directly: no
Depends on: <previous epic PR or branch>
Verification: <exact commands run>
```

## Why

Each epic builds on the previous epic's architecture and code. Stacking the work
keeps review order honest, prevents repeated `main` churn, and lets later epic
work proceed without pretending earlier epic changes have already landed on the
default branch.
