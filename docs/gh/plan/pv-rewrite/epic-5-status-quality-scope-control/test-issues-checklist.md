# Test Issues Checklist: Epic 5 - Status, Quality, And Scope Control

## Test Issue #201: Status UX Across Resource States

Labels: `test`, `priority-high`, `quality`, `control-plane`, `ready-for-agent`

Required coverage:

- [ ] Aggregate status includes project, runtime, tool, resource, gateway, daemon, and supervisor providers when available.
- [ ] Status records include desired and observed summaries.
- [ ] Normalized states cover healthy, stopped, missing install, blocked, crashed, failed, partial, and unknown.
- [ ] Failure output includes last error when present.
- [ ] Failure output includes log path when present.
- [ ] Failure output includes last reconcile time when present.
- [ ] Blocked, failed, crashed, missing install, and partial states include next action.
- [ ] Secret-like sentinel values are absent from rendered output.
- [ ] Human output is stable enough for tests.
- [ ] stdout/stderr behavior is documented and tested.
- [ ] Targeted views cover project, runtime, resource, and gateway targets.
- [ ] Missing target errors are actionable.

## Test Issue #205: Scope Guardrail And Backlog Completeness

Labels: `test`, `priority-high`, `quality`, `ready-for-agent`

Required coverage:

- [ ] `post-mvp-backlog.md` exists at the rewrite planning package root.
- [ ] Every PRD out-of-scope item is represented in the backlog or explicitly merged with another backlog item.
- [ ] Every backlog item has a deferral reason.
- [ ] Every backlog item has a reconsideration trigger.
- [ ] Backlog entries are not prerequisites for MVP issues.
- [ ] `mvp-scope-checklist.md` exists.
- [ ] The checklist asks whether new work maps to a published MVP issue.
- [ ] The checklist points deferred work back to the post-MVP backlog.
- [ ] Final QA maps MVP acceptance criteria to tests or manual QA evidence.

## Exit Evidence For All Epic 5 Test Issues

- [ ] Status tests use fake providers and deterministic clocks.
- [ ] Backlog checks run without real resources.
- [ ] Root verification passes for Go changes.
- [ ] Manual QA evidence names exact scenario, command/workflow, expected result, actual result, and follow-up issue when needed.
