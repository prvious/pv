# Drop the `services` namespace; promote rustfs and mailpit to their own packages

**Status:** Approved
**Date:** 2026-05-10

## Background

`pv` no longer uses Docker or Colima for backing services — every supervised tool now runs as a native binary. The `services` package and its sibling `svchooks` package are residue from the Docker era:

- `internal/services/` was the home of the `BinaryService` interface, the `mailpit` and `rustfs` registrations, and a few cross-cutting helpers (`ReadDotEnv`, `SanitizeProjectName`, `ServiceKey`, `WebRoute`, `ReadyCheck`/`HTTPReady`/`TCPReady`).
- `internal/svchooks/` held shared lifecycle helpers (`Install`, `Update`, `Uninstall`, `Restart`, `SetEnabled`, `PrintStatus`, `TailLog`, `WaitStopped`) keyed off `BinaryService`.

Meanwhile redis, postgres, and mysql have already been migrated to self-contained per-tool packages (`internal/redis/`, `internal/postgres/`, `internal/mysql/`) that own their lifecycle and supervision logic directly. The `manager.go` reconcile loop calls `redis.BuildSupervisorProcess()` and friends explicitly — no shared interface, no polymorphic registry.

rustfs and mailpit are the last two services still routed through the old `BinaryService` abstraction. This PR brings them to parity and removes the `services` and `svchooks` packages entirely.

## Goals

1. rustfs and mailpit each own a self-contained `internal/<tool>/` package mirroring redis/postgres/mysql.
2. `internal/services/` and `internal/svchooks/` are deleted. No "services" terminology remains in the codebase as a leftover from the Docker era.
3. The dead `Kind` registry field (a Docker-vs-binary discriminator) is removed.
4. **No user-visible changes.** All cobra commands, command aliases, `--with=` flag values, registry on-disk keys, and daemon behavior are preserved exactly.

## Non-goals

- Changing the `mail` / `s3` registry keys or `--with=` names.
- Reworking the cobra command groups themselves — `internal/commands/rustfs/` and `internal/commands/mailpit/` keep their files; their bodies just call into the new per-tool packages.
- Renaming the cobra command aliases (`s3:*`, `mail:*`).
- Migrating any other subsystem.

## Architecture

### Packages deleted

- `internal/services/` — entire package.
- `internal/svchooks/` — entire package.
- `internal/server/binary_service.go` — the polymorphic `BinaryService → supervisor.Process` adapter; replaced by per-tool `BuildSupervisorProcess()` functions.

### New packages

#### `internal/rustfs/`

Owns everything rustfs-specific. API:

- `BuildSupervisorProcess() (supervisor.Process, error)` — replaces the polymorphic adapter.
- `Install() error`, `Update() error`, `Uninstall(deleteData bool) error` — adapted from `svchooks` with no `BinaryService` argument.
- `SetEnabled(enabled bool) error`, `Restart() error` — adapted from `svchooks`.
- `PrintStatus()`, `TailLog(ctx context.Context, follow bool) error` — adapted from `svchooks`.
- `EnvVars(projectName string) map[string]string`, `Port() int`, `ConsolePort() int` — moved from the `RustFS` struct in `internal/services/rustfs.go`.
- `WebRoutes() []caddy.WebRoute` — used by caddy reverse-proxy wiring.
- `UpdateProjectEnv(projectPath, projectName string, bound *registry.ProjectServices) error` — replaces the `s3` branch of `laravel.UpdateProjectEnvForBinaryService`.

#### `internal/mailpit/`

Same shape as `internal/rustfs/`, with mailpit's specifics (port 1025, console port 8025, HTTP `/livez` ready check, no per-project env vars, no `EnvVars` parameterization).

#### `internal/projectenv/`

Single-purpose home for the cross-cutting helpers that previously lived in `internal/services/`:

- `ReadDotEnv(path string) (map[string]string, error)` — reads `.env`-style files.
- `SanitizeProjectName(name string) string` — alphanumeric + underscore identifier sanitizer.

Used by `cmd/link.go`, `cmd/install.go`, `internal/commands/postgres/install.go`, `internal/commands/mysql/install.go`, `internal/automation/steps/detect_services.go`.

### Type relocations

Types that previously lived in `internal/services/` migrate to their natural consumers:

| Type / function | New home | Rationale |
|---|---|---|
| `WebRoute` | `internal/caddy/` | Only consumer is caddy's reverse-proxy generator. |
| `ReadyCheck`, `HTTPReady`, `TCPReady` | `internal/supervisor/` | They produce `func(ctx) error`, which is the exact shape the supervisor consumes. |
| `ServiceKey`, `ParseServiceKey` | `internal/registry/` | Versioned-key parsing belongs with registry storage. |
| `Available`, `Lookup`, `LookupBinary`, `AllBinary` | (deleted) | Polymorphic enumeration is gone — see "Cross-cutting callers" below. |

### Cross-cutting callers

The polymorphic `services.AllBinary()` iteration is replaced with explicit per-tool calls everywhere, mirroring how redis/postgres/mysql are already wired.

#### `internal/server/manager.go`

In `reconcileBinaryServices`, the "Source 1" loop walking `services.AllBinary()` is replaced with two explicit blocks:

```go
// Source 1a — rustfs.
if entry := reg.Services["s3"]; entry != nil && enabled(entry) {
    proc, err := rustfs.BuildSupervisorProcess()
    if err != nil {
        startErrors = append(startErrors, fmt.Sprintf("s3: build: %v", err))
    } else {
        wanted[binaries.Rustfs.Name] = proc
    }
}

// Source 1b — mailpit.
if entry := reg.Services["mail"]; entry != nil && enabled(entry) {
    proc, err := mailpit.BuildSupervisorProcess()
    if err != nil {
        startErrors = append(startErrors, fmt.Sprintf("mail: build: %v", err))
    } else {
        wanted[binaries.Mailpit.Name] = proc
    }
}
```

The existing redis/postgres/mysql blocks immediately below remain unchanged; this matches their pattern exactly.

#### `internal/caddy/caddy.go`

The `services.LookupBinary(svcName)` lookup becomes a switch on the bound service name:

```go
var routes []WebRoute
switch svcName {
case "s3":
    routes = rustfs.WebRoutes()
case "mail":
    routes = mailpit.WebRoutes()
default:
    continue
}
```

#### `internal/laravel/env.go`

`UpdateProjectEnvForBinaryService(svc services.BinaryService, ...)` is removed. Its callers (currently `cmd/link.go` and the bind-on-add path) switch on the service name and call `rustfs.UpdateProjectEnv(...)` / `mailpit.UpdateProjectEnv(...)` directly.

#### `cmd/install.go`, `cmd/setup.go`, `cmd/update.go`

The `--with=` flag validation, the setup-wizard "available services" list, and the update loop all previously called `services.Available()` / `services.LookupBinary()` / `services.AllBinary()`. They become a single hardcoded slice plus dispatching helpers, both unexported in `cmd/install.go` (the primary consumer; `setup.go` and `update.go` reach in via the same package):

```go
// cmd/install.go
var binaryAddons = []string{"s3", "mail"}

func installAddon(name string) error {
    switch name {
    case "s3":
        return rustfs.Install()
    case "mail":
        return mailpit.Install()
    default:
        return fmt.Errorf("unknown addon %q (available: %s)", name, strings.Join(binaryAddons, ", "))
    }
}
```

(Same pattern for update and uninstall paths.)

### `Kind` field removal

`registry.ServiceInstance.Kind` is removed entirely. It was a Docker-vs-binary discriminator and every check is now a tautology:

- The `Kind string` field is deleted from the struct.
- All `if Kind != "binary"` guards (in `manager.go`, `svchooks/install.go`, `svchooks/wait.go`) go away — they were either dead checks or part of svchooks files being deleted outright.
- All `Kind = "binary"` writes go away.
- The "Kind: binary" line in the status output (`svchooks/status.go`) is dropped — moved code in `internal/{rustfs,mailpit}/PrintStatus()` simply omits it.

**Backwards compatibility:** Existing on-disk registries containing `"kind": "binary"` will continue to deserialize cleanly because Go's `encoding/json` silently ignores unknown JSON keys. The first registry save after upgrade rewrites without the field. No migration code is needed.

## Test plan

### Test redistribution

Existing tests move with their subjects:

| Current location | New location |
|---|---|
| `internal/services/rustfs_test.go` | `internal/rustfs/rustfs_test.go` |
| `internal/services/mailpit_test.go` | `internal/mailpit/mailpit_test.go` |
| `internal/services/dotenv_test.go` | `internal/projectenv/dotenv_test.go` |
| `internal/services/binary_test.go` | (deleted — covers the removed `BinaryService` interface) |
| `internal/services/service_test.go` | Split: `SanitizeProjectName` tests → `internal/projectenv/`; `ServiceKey`/`ParseServiceKey` tests → `internal/registry/`. |
| `internal/svchooks/lifecycle_test.go` | Split: rustfs cases → `internal/rustfs/lifecycle_test.go`; mailpit cases → `internal/mailpit/lifecycle_test.go`. |
| `internal/svchooks/svchooks_test.go` | Deleted — covers `UpdateLinkedProjectsEnvBinary` which becomes per-tool tests under each tool's package. |
| `internal/server/binary_service_test.go` | Ready-check tests follow `HTTPReady`/`TCPReady` to `internal/supervisor/`. |

### New tests

- `internal/rustfs/` and `internal/mailpit/`: unit tests asserting their `BuildSupervisorProcess()` returns a process whose `Name`, `Cmd`, `Args`, and `ReadyCheck` match expectations (porting from the existing service-level tests).
- A registry-load test asserting an on-disk registry with a legacy `"kind": "binary"` JSON field still parses without error and the field is dropped on next save.

### Existing coverage to keep green

- `cmd/install_test.go`, `cmd/link_test.go`, `cmd/uninstall_test.go`, `cmd/setup_test.go` — the rewritten `--with=`/binding/install/uninstall switches must keep these passing without modification.
- `internal/server/manager*_test.go` — reconcile-loop tests for rustfs + mailpit must continue to assert the same supervisor wiring.
- `scripts/e2e/` mailpit and rustfs phases on CI — the user-facing `mailpit:*` / `rustfs:*` / `mail:*` / `s3:*` commands must produce identical output and side effects.

## Migration / rollout

This is purely an internal refactor. No data migration; no user-facing command changes; no CI workflow changes; no release-note action item beyond "internal cleanup".

The PR will be one commit (or a small handful) and dispatch a targeted CI run. Per `CLAUDE.md` dispatch conventions, since the change touches no artifact-build logic, the routine `go build && go test ./...` plus the rustfs and mailpit e2e phases are sufficient.

## Risks

1. **Hardcoded addon list drift.** With `services.AllBinary()` gone, adding a third binary-backed addon later means touching `manager.go`, `caddy.go`, `laravel/env.go`, `cmd/install.go`, `cmd/setup.go`, `cmd/update.go`. This is an explicit trade-off — chosen for parity with the redis/pg/mysql wiring, which already has the same characteristic. Mitigation: a single canonical `binaryAddons` slice with dispatch helpers in `cmd/install.go` keeps the touch points predictable.
2. **Stale registries on rollback.** If a user downgrades to a pre-PR binary, an upgraded-and-resaved registry will be missing the `"kind"` field. The old binary's `Kind == ""` path treats that as "docker" (per the existing default-empty comment), which would mis-classify binary-backed entries. This is acceptable: rollback is a manual operation, and pv has no documented downgrade flow. Pre-PR users running upgrades forward have no issue (unknown JSON keys are ignored).

## Out of scope (potential follow-ups)

- Renaming the `mail` / `s3` registry keys to `mailpit` / `rustfs` (intentionally deferred — would require a registry migration for existing users).
- Removing the `mail:*` / `s3:*` cobra aliases (kept for muscle memory).
- Any further `internal/server/manager.go` deduplication of the four-source reconcile loop.
