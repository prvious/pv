# Test Strategy: Epic 1 - Rewrite Foundation

## Scope

Epic 1 tests cover:

- prototype relocation;
- root module scaffold;
- minimal scriptable CLI behavior;
- desired and observed store seam;
- first installable resource controller;
- status output for pending, ready, and failed states.

## Test Objectives

- Prove the prototype remains buildable as reference-only code.
- Prove the root module builds independently.
- Prove command behavior is scriptable from the first slice.
- Prove desired state and observed status are distinct.
- Prove commands request state changes instead of performing orchestration.
- Prove a controller can reconcile state and report observed status.

## ISTQB Techniques

| Technique | Epic 1 usage |
| --- | --- |
| Equivalence partitioning | Valid version strings, invalid version strings, known commands, unknown commands. |
| Boundary value analysis | Empty args, missing version, empty state file, missing observed status. |
| Decision table testing | Desired present/absent and observed present/absent status output. |
| State transition testing | desired -> pending observed -> ready or failed observed. |
| Experience-based testing | Avoid old prototype coupling and hidden command work. |

## Test Matrix

| Area | Required tests |
| --- | --- |
| Prototype relocation | Prototype module builds and tests from `legacy/prototype`. |
| Root scaffold | Root module builds and tests from repository root. |
| CLI | Help, version, unknown command, stdout/stderr separation. |
| Store | Desired persistence, observed persistence, separation, invalid versions. |
| Controller | No desired no-op, success ready status, failure status and next action. |
| Status | No desired, desired pending, ready observed, failed observed. |

## Test Data

- Use `t.TempDir()` for state files.
- Use deterministic clocks for observed status timestamps.
- Use fake installers or marker installers only.
- Do not download real artifacts.
- Do not mutate the user's real `HOME`.

## Verification Commands

Root:

```bash
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

Prototype, if moved or changed:

```bash
cd legacy/prototype
gofmt -w .
go vet ./...
go build ./...
go test ./...
```

## Test Issue Contract

Use `test-issues-checklist.md` as the execution checklist. Epic 1 has exactly
two test issues:

- #122 validates prototype/root buildability and CLI scriptability.
- #127 validates desired/observed tracer behavior.

No Epic 1 test may download real artifacts, start long-running processes, or
exercise Laravel/PHP/service behavior.

## Exit Criteria

- All Epic 1 tests pass.
- Root verification passes.
- Prototype verification passes when applicable.
- PR body lists exact commands.
- No tests rely on real downloads or external services.
