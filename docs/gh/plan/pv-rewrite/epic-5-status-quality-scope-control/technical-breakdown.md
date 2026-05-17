# Technical Breakdown: Epic 5 - Status, Quality, And Scope Control

## Module Roles

| Module | Responsibility |
| --- | --- |
| `internal/status` | Aggregate status model, normalized states, metadata, and provider contracts. |
| `internal/console` | Stable human rendering and stderr/stdout separation. |
| Resource/project packages | Status providers that return data, not rendered UI. |
| `docs/gh/plan/pv-rewrite/post-mvp-backlog.md` | Deferred capabilities with reasons and triggers. |
| `docs/gh/plan/pv-rewrite/mvp-scope-checklist.md` | Review-time MVP boundary checks. |

## Normalized States

| State | Meaning |
| --- | --- |
| `healthy` | Desired and observed state match. |
| `stopped` | Desired resource exists but process is not running. |
| `missing_install` | Desired resource needs an install that is absent. |
| `blocked` | A prerequisite is missing or failed. |
| `crashed` | A runnable process exited unexpectedly. |
| `failed` | Reconcile attempted and failed. |
| `partial` | Some requested parts reconciled and others did not. |
| `unknown` | Provider cannot determine status. |

## Status Record Shape

Each status record includes:

- stable ID;
- resource or project kind;
- desired summary;
- observed summary;
- normalized state;
- log path when available;
- last error when available;
- last reconcile time when available;
- next action for blocked, failed, crashed, missing install, and partial states;
- redaction marker for secret-like fields.

## Command Contract

- `pv status` shows aggregate status.
- `pv status project <name-or-path>` shows one project and related resource failures.
- `pv status runtime <name>` shows runtime/tool status.
- `pv status resource <kind> [name]` shows declared backing resources.
- `pv status gateway [host]` shows gateway route and TLS/DNS/process status.

Targeted views reuse aggregate data and must not hide failures relevant to the requested target.

## Scope Control Files

- `post-mvp-backlog.md` lists each deferred capability with reason and trigger.
- `mvp-scope-checklist.md` is used during issue/PR review.
- `issue-label-audit.md` records the label audit for #116-#205.

## Non-Goals

- No TUI-first status experience.
- No implicit machine-readable output; machine output is added only with an explicit flag and tests.
- No deferred backlog item becomes MVP scope without a planning update.
