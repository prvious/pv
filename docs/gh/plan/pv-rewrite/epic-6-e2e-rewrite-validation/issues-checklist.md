# Issues Checklist: Epic 6 - E2E Rewrite Validation

Published after Epic 5 and tracked by Epic 6 PR #212.

## Published Issues

Milestone: `pv rewrite MVP`

| Type | Issue | Title |
| --- | --- | --- |
| Epic | #213 | Epic: E2E Rewrite Validation |
| Feature | #214 | Feature: E2E Harness And Fixtures |
| Feature | #215 | Feature: Laravel Project Lifecycle E2E |
| Feature | #216 | Feature: Resource Failure And Recovery E2E |
| Feature | #217 | Feature: CI And Release Gates |
| Enabler | #218 | Enabler: Build pv binary for E2E |
| Enabler | #219 | Enabler: Add sandbox and command runner |
| Enabler | #220 | Enabler: Add Laravel fixture generator |
| Test | #221 | Test: Harness Isolation And Cleanup |
| User Story | #222 | User Story: Validate pv init lifecycle |
| User Story | #223 | User Story: Validate pv link env setup lifecycle |
| User Story | #224 | User Story: Validate status and helper workflows |
| Test | #225 | Test: Laravel Lifecycle E2E |
| User Story | #226 | User Story: Validate missing install and blocked dependency failures |
| User Story | #227 | User Story: Validate setup process and gateway failures |
| User Story | #228 | User Story: Validate recovery after corrective action |
| Test | #229 | Test: Failure And Recovery E2E |
| Enabler | #230 | Enabler: Define E2E tiers and CI-only controls |
| Enabler | #231 | Enabler: Extend tests workflow with E2E jobs |
| User Story | #232 | User Story: Record E2E evidence for release |
| Test | #233 | Test: CI And Release Gate Behavior |

Tracker hygiene performed:

- Created label `e2e`.
- Added milestone `pv rewrite MVP` to Epic 6 issues #213-#233.
- Added `ready-for-agent` to Epic 6 leaf issues #218-#233.
- Linked container issue bodies to child issues.
- Linked all issue bodies and PR #212.

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
- [ ] Local E2E does not mutate real host state or download artifacts.
- [ ] Tier 1 and Tier 2 run in GitHub-hosted CI VMs and refuse local execution.
- [ ] `.github/workflows/tests.yml` is the single normal plus E2E workflow.
- [ ] E2E release evidence is recorded before MVP release readiness.

## Definition Of Done

- [ ] Feature issues complete.
- [ ] Test issues complete.
- [ ] Tier 0 E2E release gate passes.
- [ ] Root verification passes for Go changes.
- [ ] Tier 1/Tier 2 CI-only refusal behavior is documented.
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
| Enabler | Define E2E tiers and CI-only controls | `enabler`, `priority-high`, `quality`, `e2e` | 2 | Tier 0 default exists; Tier 1 and Tier 2 run in GitHub-hosted CI VMs and refuse local execution. |
| Enabler | Extend tests workflow with E2E jobs | `enabler`, `priority-high`, `quality`, `e2e` | 3 | `tests.yml` runs normal checks plus E2E tier jobs. |
| Story | Record E2E evidence for release | `user-story`, `priority-high`, `quality`, `e2e` | 2 | Evidence includes scenarios, commands, expected/actual results, logs, and follow-up issues. |
| Test | CI and release gate behavior | `test`, `priority-high`, `quality`, `e2e` | 3 | Gate command, CI jobs, and local refusal protections are tested. |

## Test Issues

Use `test-issues-checklist.md` as the source of truth for test acceptance.

- [ ] Test: Harness Isolation And Cleanup
- [ ] Test: Laravel Lifecycle E2E
- [ ] Test: Failure And Recovery E2E
- [ ] Test: CI And Release Gate Behavior

## Publishing Notes

- Label `e2e` exists and is applied to Epic 6 issues.
- `ready-for-agent` is applied only to leaf issues after issue bodies are reviewed.
- Epic 6 implementation PRs target `rewrite/epic-6-e2e-rewrite-validation`, based on `rewrite/epic-5-status-quality-scope`.
- No Epic 6 PR targets `main` directly.
