# Test Strategy: Epic 5 - Status, Quality, And Scope Control

## Scope

Epic 5 tests cover:

- aggregate status model and provider shape;
- desired and observed status rendering;
- failure, log path, last error, last reconcile time, and next action output;
- scriptable output behavior;
- targeted status views;
- secret redaction;
- post-MVP backlog completeness;
- MVP scope checklist coverage.

## Test Objectives

- Prove status output explains both desired and observed state.
- Prove status helps users recover from blocked and failed states.
- Prove output remains stable enough for scripting.
- Prove secret-like values are redacted.
- Prove deferred work is tracked without expanding MVP scope.
- Prove final QA maps requirements to concrete evidence.

## ISTQB Techniques

| Technique | Epic 5 usage |
| --- | --- |
| Equivalence partitioning | Healthy/stopped/missing/blocked/crashed/failed/partial/unknown status groups. |
| Boundary value analysis | No resources, one project, missing log path, empty last error, absent next action. |
| Decision table testing | Status output across desired/observed combinations and failure metadata presence. |
| State transition testing | pending -> healthy, healthy -> crashed, blocked -> ready, partial -> failed. |
| Experience-based testing | Avoid decorative status, brittle output, leaked secrets, and silent MVP scope expansion. |

## Test Matrix

| Area | Required tests |
| --- | --- |
| Aggregate model | Desired/observed fields, providers, normalized states, metadata. |
| Resource/project status | Healthy, stopped, missing install, blocked, crashed, failed, partial, unknown. |
| Failure output | Last error, log path, last reconcile time, next action. |
| Scriptability | Stable human output, stderr/stdout behavior, explicit machine output if added. |
| Targeted views | Project, runtime, resource, gateway targets, missing target errors. |
| Redaction | Secret-like values in env, credentials, and URLs are not printed. |
| Backlog | Omitted items, deferral reasons, reconsideration triggers. |
| Scope checklist | Review checklist exists and points out-of-scope work to backlog. |

## Test Data

- Use fake status providers for each resource family.
- Use deterministic clocks for last reconcile time.
- Use secret-like sentinel values and assert absence in rendered output.
- Use sample backlog entries that include and omit required fields to verify QA
  checks.
- Do not require real resource processes.

## Integration Coverage

Minimum integration checks when implementations are ready:

1. `pv status` aggregates at least one project, runtime, gateway, and resource.
2. Blocked Composer or missing resource status includes next action.
3. Crashed runnable resource status includes log path and last error.
4. Targeted status view returns one requested project or resource.
5. Backlog QA check passes against the planning package.

## Verification Commands

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Test Issue Contract

Use `test-issues-checklist.md` as the execution checklist. Epic 5 has exactly
two test issues:

- #201 validates aggregate and targeted status UX across normalized states.
- #205 validates backlog completeness and MVP scope guardrails.

Targeted status views for MVP are exactly project, runtime, resource, and
gateway.

## Exit Criteria

- All Epic 5 tests pass.
- Status covers representative state transitions and failure metadata.
- Scriptable output behavior is documented and tested.
- Post-MVP backlog and MVP scope checklist are complete.
