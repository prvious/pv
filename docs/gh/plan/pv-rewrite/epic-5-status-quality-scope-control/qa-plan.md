# QA Plan: Epic 5 - Status, Quality, And Scope Control

## Quality Gates

| Gate | Required Evidence |
| --- | --- |
| Status model | Desired and observed state are represented with normalized states. |
| Failure UX | Log paths, errors, timestamps, and next actions are shown where available. |
| Scriptability | stdout/stderr behavior is stable and documented. |
| Redaction | Secret-like values are absent from rendered status. |
| Targeted views | Targeted status views reuse aggregate data and produce actionable errors. |
| Backlog | Each deferred capability has reason and trigger. |
| Final QA | MVP acceptance criteria map to tests or manual QA evidence. |

## Manual QA Checklist

- [ ] Run aggregate `pv status` with fake or real project/resource states.
- [ ] Confirm healthy, blocked, failed, and partial states are distinguishable.
- [ ] Confirm status includes log path and next action when available.
- [ ] Confirm secret-like values do not appear in output.
- [ ] Run targeted status for a project or resource.
- [ ] Confirm missing target errors are actionable.
- [ ] Review post-MVP backlog for deferral reason and trigger on every entry.
- [ ] Review MVP scope checklist before final release branch.

## Review Checklist

- [ ] Status output is not only decorative.
- [ ] Status providers do not render their own UI.
- [ ] Human output and pipeable output are intentionally separated.
- [ ] Secret redaction tests use sentinel values.
- [ ] Deferred backlog items are not prerequisites for MVP issues.
- [ ] Scope checklist is referenced from planning docs.
- [ ] Final QA evidence covers all epic acceptance criteria.

## Required Verification

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

If final QA relies on manual checks, document:

- exact scenario;
- command or workflow;
- expected result;
- actual result;
- follow-up issue if the result fails.
