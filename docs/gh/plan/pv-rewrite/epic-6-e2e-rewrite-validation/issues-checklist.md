# Issues Checklist: Epic 6 - E2E Rewrite Validation

Create these issues after Epic 5 is published or when the rewrite stack reaches
the E2E validation branch.

## Publishing Status

Milestone: `pv rewrite MVP`

Issue numbers are unpublished. Add concrete issue numbers after publishing.

## Epic Issue

### Title

`Epic: E2E Rewrite Validation`

### Labels

`epic`, `priority-critical`, `value-high`, `quality`, `e2e`

### Body

```markdown
## Epic Description

Add black-box end-to-end validation for the new pv rewrite. The suite builds and
invokes the active rewrite binary, runs in a sandbox, validates Laravel-first MVP
workflows, checks failures and recovery, and defines the release E2E gate.

## Business Value

- Maintainers get integrated release evidence.
- User-visible behavior is tested through the CLI, files, logs, status, and exit codes.
- Default E2E runs stay safe for local machines and CI.

## Features

- [ ] Feature: E2E Harness And Fixtures
- [ ] Feature: Laravel Project Lifecycle E2E
- [ ] Feature: Resource Failure And Recovery E2E
- [ ] Feature: CI And Release Gates

## Acceptance Criteria

- [ ] Tier 0 E2E suite builds and invokes active rewrite binary.
- [ ] Tier 0 E2E suite runs in isolated temp HOME and project roots.
- [ ] Init, link, status, gateway, helper, failure, and recovery workflows are covered.
- [ ] Default E2E does not mutate real host state or download artifacts.
- [ ] Tier 1 and Tier 2 checks are opt-in.
- [ ] Tier 0 release gate passes before MVP release readiness.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Tier 0 E2E release gate passes.
- [ ] Root verification passes for Go changes.
- [ ] Tier 1/Tier 2 opt-in behavior is documented.
```

## Feature Issues

| Feature | Labels | Estimate | Blocked by |
| --- | --- | --- | --- |
| E2E Harness And Fixtures | `feature`, `priority-critical`, `value-high`, `quality`, `e2e` | 8 | Epic 5 stack branch |
| Laravel Project Lifecycle E2E | `feature`, `priority-critical`, `value-high`, `quality`, `e2e`, `laravel` | 8 | E2E Harness And Fixtures |
| Resource Failure And Recovery E2E | `feature`, `priority-critical`, `value-high`, `quality`, `e2e` | 8 | E2E Harness And Fixtures |
| CI And Release Gates | `feature`, `priority-high`, `value-high`, `quality`, `e2e` | 5 | lifecycle and failure E2E tests |

## Story And Enabler Issues

| Type | Title | Labels | Estimate | Acceptance criteria |
| --- | --- | --- | --- | --- |
| Enabler | Build pv binary for E2E | `enabler`, `priority-critical`, `quality`, `e2e` | 2 | Active rewrite binary is built into a temp path; legacy prototype binary is not used. |
| Enabler | Add sandbox and command runner | `enabler`, `priority-critical`, `quality`, `e2e` | 3 | HOME, pv state, project root, logs, ports, stdout, stderr, and exit code are isolated and captured. |
| Enabler | Add Laravel fixture generator | `enabler`, `priority-critical`, `quality`, `e2e`, `laravel` | 3 | Minimal Laravel fixture is deterministic and supports init/link scenarios. |
| Test | Harness isolation and cleanup | `test`, `priority-critical`, `quality`, `e2e` | 3 | Harness tests prove temp roots, cleanup, and no real `~/.pv` use. |
| Story | Validate pv init lifecycle | `user-story`, `priority-critical`, `quality`, `e2e`, `laravel` | 3 | Fresh init, overwrite refusal, force overwrite, and no `.env` mutation are covered. |
| Story | Validate pv link env setup lifecycle | `user-story`, `priority-critical`, `quality`, `e2e`, `laravel` | 5 | Link records desired state, writes declared env, runs setup, and signals after durable state. |
| Story | Validate status and helper workflows | `user-story`, `priority-critical`, `quality`, `e2e`, `laravel` | 5 | Status and helpers work through public CLI behavior and declared resources. |
| Test | Laravel lifecycle E2E | `test`, `priority-critical`, `quality`, `e2e`, `laravel` | 5 | Init, link, status, and helper E2E scenarios pass. |
| Story | Validate missing install and blocked dependency failures | `user-story`, `priority-critical`, `quality`, `e2e` | 3 | Missing prerequisites produce error, next action, and blocked status. |
| Story | Validate setup process and gateway failures | `user-story`, `priority-critical`, `quality`, `e2e` | 5 | Setup, process, and gateway failures include logs and next actions. |
| Story | Validate recovery after corrective action | `user-story`, `priority-critical`, `quality`, `e2e` | 5 | Corrective action clears blocked/failed state in follow-up status. |
| Test | Failure and recovery E2E | `test`, `priority-critical`, `quality`, `e2e` | 5 | Failure and recovery scenario coverage passes. |
| Enabler | Define E2E tiers and opt-in controls | `enabler`, `priority-high`, `quality`, `e2e` | 2 | Tier 0 default, Tier 1 opt-in, and Tier 2 opt-in controls exist. |
| Enabler | Add release gate command | `enabler`, `priority-high`, `quality`, `e2e` | 3 | Tier 0 command is documented, scriptable, and fails closed. |
| Story | Record E2E evidence for release | `user-story`, `priority-high`, `quality`, `e2e` | 2 | Evidence includes scenarios, commands, expected/actual results, logs, and follow-up issues. |
| Test | CI and release gate behavior | `test`, `priority-high`, `quality`, `e2e` | 3 | Gate command and opt-in protections are tested. |

## Test Issues

Use `test-issues-checklist.md` as the source of truth for test acceptance.

- [ ] Test: Harness Isolation And Cleanup
- [ ] Test: Laravel Lifecycle E2E
- [ ] Test: Failure And Recovery E2E
- [ ] Test: CI And Release Gate Behavior

## Publishing Notes

- Add label `e2e` before publishing Epic 6 issues.
- Add `ready-for-agent` only to leaf issues after issue bodies are reviewed.
- Epic 6 implementation PRs target `rewrite/epic-6-e2e-rewrite-validation`, based on `rewrite/epic-5-status-quality-scope`.
- No Epic 6 PR targets `main` directly.
