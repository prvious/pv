# Technical Breakdown: Epic 1 - Rewrite Foundation

## Module Roles

| Module | Responsibility |
| --- | --- |
| `main.go` | Call the active root CLI and return process exit codes. |
| `internal/cli` | Parse Epic 1 commands, enforce stdout/stderr behavior, and call use-case seams. |
| `internal/control` | Define desired resource state, observed status, store interface, and status states. |
| `internal/resources/mago` | Reconcile the Mago tracer desired state with an installer adapter. |
| `legacy/prototype` | Buildable reference-only prototype module. |

## Data Authority

| Data | Authority | Epic 1 representation |
| --- | --- | --- |
| Prototype code | `legacy/prototype` | Separate Go module. |
| Desired tracer request | Store | Resource name and requested version. |
| Observed tracer status | Store | Resource, desired version, state, timestamp, error, next action. |
| CLI output | CLI | stdout for pipeable data, stderr for human status/errors. |

## Required Flows

### `mago:install <version>`

1. CLI validates exactly one version argument.
2. CLI rejects unsafe or empty version strings.
3. CLI writes desired state only.
4. CLI writes a concise human confirmation to stderr.
5. CLI does not call installer or controller code.

### Mago controller reconcile

1. Controller reads desired state for Mago.
2. Controller no-ops when no desired state exists.
3. Controller calls the installer adapter when desired state exists.
4. Success writes `ready` observed status.
5. Failure writes `failed` observed status with last error and next action.

### `status`

1. Status reads desired and observed records separately.
2. No desired state prints a no-request state.
3. Desired without observed prints pending reconcile.
4. Ready and failed observed states include the requested version.
5. Failed status includes next action.

## Non-Goals

- No PHP or Composer implementation.
- No daemon or supervisor.
- No SQLite store.
- No real artifact downloads.
- No Laravel project contract, gateway, or backing services.

## Test Seams

- Store tests use `t.TempDir()` and deterministic fixtures.
- Installer tests use fake or marker installers only.
- Status tests assert stable semantic lines, not decorative spacing.
