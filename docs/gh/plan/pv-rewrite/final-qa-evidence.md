# Final QA Evidence

This file records the MVP review evidence shape for the stacked rewrite PRs.
It is intentionally short so each epic PR can keep its own command evidence in
the PR body.

| Gate | Evidence |
| --- | --- |
| Stack shape | `docs/gh/plan/pv-rewrite/stacked-diff-plan.md` names the five branch levels and base branches. |
| Scope guardrail | `docs/gh/plan/pv-rewrite/mvp-scope-checklist.md` and `post-mvp-backlog.md` define MVP boundaries and deferred capabilities. |
| Status quality | `internal/status` covers normalized states, targeted views, log/error/next-action fields, and redaction. |
| Root verification | Each epic PR body must list the exact root verification commands run. |
| CI | Each epic PR must show a successful `E2E Tests / e2e` check before handoff. |
