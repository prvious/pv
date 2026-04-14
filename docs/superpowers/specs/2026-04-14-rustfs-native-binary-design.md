# RustFS as a Native Supervised Binary (S3 Service)

**Date:** 2026-04-14
**Status:** Approved

## Problem

The `s3` service currently runs RustFS via Docker (`rustfs/rustfs:latest`), which forces users to install and run Colima — a second Linux VM on macOS for any user who already has Docker Desktop or OrbStack. The Docker container engine is also still a stub (see `docs/superpowers/plans/2026-03-25-docker-container-engine.md`), so `pv service:add s3` doesn't actually produce a working S3 endpoint today.

RustFS distributes as a single static binary. Running it directly — supervised by the pv daemon — removes the Colima dependency for S3, eliminates a ~2 GB RAM VM overhead, cuts cold-start from ~30 s to sub-second, and deletes a large amount of container-orchestration scaffolding from this service's code path.

This spec establishes RustFS as the first binary-backed service, and lays down the `BinaryService` interface and `supervisor` package that mail / redis / future services will plug into later.

## Goals

- S3 (RustFS) runs as a native binary with no Docker/Colima dependency.
- Supervised by the pv daemon (same process as FrankenPHP); lives and dies with it.
- User-facing CLI (`pv service:add s3`, `service:start s3`, etc.) keeps working with the same mental model.
- Establish a reusable `BinaryService` interface so mail and redis can migrate later by implementing one interface, not by reinventing the pattern.

## Non-Goals

- Do **not** touch the Docker code path for mail / mysql / postgres / redis. They remain Docker-backed in this change.
- Do **not** refactor the existing `Service` interface. It stays frozen; `BinaryService` is parallel.
- Do **not** build cross-service IPC. Daemon auto-restart is the mechanism for picking up service state changes.
- Do **not** generate per-install credentials. Keep the existing hardcoded `rstfsadmin/rstfsadmin` for continuity.
- Do **not** add log rotation, resource limits, or structured health probes beyond "process is alive and port responds once at startup."

## Architecture

### New packages / files

| Path | Purpose |
|------|---------|
| `internal/services/binary.go` | `BinaryService` interface; `ReadyCheck` type; `binaryRegistry`; `LookupBinary`; `AllBinary` |
| `internal/services/rustfs.go` | `RustFS` struct implementing `BinaryService` (registered under `"s3"`) |
| `internal/binaries/rustfs.go` | Platform-specific archive name + download URL (mirrors `binaries/mago.go`) |
| `internal/supervisor/supervisor.go` | Child-process supervisor: spawn, ready-wait, crash-restart, graceful-stop |
| `internal/supervisor/supervisor_test.go` | Unit tests using a `//go:build testhelper` tagged helper binary |
| `internal/commands/service/dispatch.go` | Shared `resolveKind` helper used by every `service:*` command |
| `internal/commands/service/restart.go` | `autoRestart(ctx)` helper — daemon-restart-if-running used by every binary-service mutation |
| `scripts/e2e/s3-binary.sh` | New e2e phase that exercises the full binary flow |

### Modified files

| Path | Change |
|------|--------|
| `internal/services/service.go` | Narrow `registry` (Docker-only) to mail/mysql/postgres/redis; add `LookupDocker`, `Lookup(kind)`; `Available()` returns union |
| `internal/binaries/manager.go` | Add `"rustfs":` cases in `DownloadURL` and `LatestVersionURL`. Do **not** add Rustfs to `Tools()` (it's a backing service, not a user-exposed tool) |
| `internal/registry/registry.go` | `ServiceInstance` gains `Kind string` and `Enabled *bool` fields (both JSON `omitempty`; missing defaults preserve current Docker behavior) |
| `internal/server/process.go` | `Start()` instantiates `supervisor.Supervisor`, spawns registered+enabled binary services, writes `daemon-status.json`, uses `defer sup.StopAll(10*time.Second)` |
| `internal/commands/service/add.go` | Use `resolveKind`; binary path downloads + registers + `autoRestart` |
| `internal/commands/service/start.go` | Binary path: set `Enabled=true`, `autoRestart` |
| `internal/commands/service/stop.go` | Binary path: set `Enabled=false`, `autoRestart` |
| `internal/commands/service/remove.go` | Binary path: unregister + delete binary + `autoRestart` |
| `internal/commands/service/destroy.go` | Binary path: `remove` + delete `~/.pv/data/s3/` |
| `internal/commands/service/status.go` | Read `~/.pv/daemon-status.json` for binary services; fall back to "registered, not running" if daemon down |
| `internal/commands/service/list.go` | Merged table across Docker + binary services |
| `internal/commands/service/logs.go` | Binary path: tail `~/.pv/logs/rustfs.log` |
| `cmd/update.go` | After tool updates, iterate `services.AllBinary()` for registered entries and refresh binaries; advise `pv restart` if newer version was fetched while daemon running |
| `.github/workflows/e2e.yml` | Add phase invoking `scripts/e2e/s3-binary.sh` |

### Interfaces

```go
// internal/services/binary.go

type ReadyCheck struct {
    TCPPort      int           // probe 127.0.0.1:port until Dial succeeds
    HTTPEndpoint string        // GET this URL, expect 2xx
    Timeout      time.Duration // give up after this (per check invocation)
}
// Exactly one of TCPPort or HTTPEndpoint must be set.

type BinaryService interface {
    Name() string
    DisplayName() string
    Binary() binaries.Binary
    Args(dataDir string) []string
    Env() []string
    Port() int
    ConsolePort() int
    WebRoutes() []WebRoute
    EnvVars(projectName string) map[string]string
    ReadyCheck() ReadyCheck
}
```

The existing `Service` (Docker) interface is unchanged.

## Components

### `RustFS` implementation (`internal/services/rustfs.go`)

- `Name()`: `"s3"` (matches the user-facing service name)
- `DisplayName()`: `"S3 Storage (RustFS)"`
- `Binary()`: `binaries.Rustfs`
- `Args(dataDir)`: `["server", dataDir, "--address", ":9000", "--console-address", ":9001"]`
  - **VERIFY during implementation:** exact flag names by running `./rustfs server --help` on the downloaded binary.
- `Env()`: `["RUSTFS_ROOT_USER=rstfsadmin", "RUSTFS_ROOT_PASSWORD=rstfsadmin"]`
- `Port()`: `9000`; `ConsolePort()`: `9001`
- `WebRoutes()`: `[{s3 → 9001}, {s3-api → 9000}]` (unchanged from current Docker version)
- `EnvVars(project)`: identical keys/values to the current Docker S3 so linked projects keep working
- `ReadyCheck()`: `TCPPort: 9000, Timeout: 30s`

### `binaries.Rustfs` (`internal/binaries/rustfs.go`)

- Archive naming: `rustfs-{platform}-latest.zip`, where `{platform}` is:
  - `darwin/arm64` → `macos-aarch64` (confirmed by user's curl)
  - `darwin/amd64` → `macos-x86_64` (**VERIFY** on releases page)
  - `linux/amd64` → `linux-x86_64` (**VERIFY**)
  - `linux/arm64` → `linux-aarch64` (**VERIFY**)
- Download URL pattern: `https://github.com/rustfs/rustfs/releases/download/{version}/rustfs-{platform}-latest.zip`
- Latest version: `https://api.github.com/repos/rustfs/rustfs/releases/latest`
- `NeedsExtract: true` — `.zip` archive, extract to `~/.pv/internal/bin/rustfs`
- **VERIFY during implementation:** whether the `.zip` contains `rustfs` at the root or inside a subdirectory.

### `supervisor` package (`internal/supervisor/`)

```go
type Process struct {
    Name         string
    Binary       string
    Args         []string
    Env          []string
    WorkingDir   string
    LogFile      string
    Ready        func(ctx context.Context) error
    ReadyTimeout time.Duration
}

type Supervisor struct { /* mu, processes map */ }

func New() *Supervisor
func (s *Supervisor) Start(ctx context.Context, p Process) error
func (s *Supervisor) Stop(name string, timeout time.Duration) error
func (s *Supervisor) StopAll(timeout time.Duration) error
func (s *Supervisor) IsRunning(name string) bool
func (s *Supervisor) Pid(name string) int
```

**Lifecycle:**

1. `Start(ctx, p)`:
   - Open `p.LogFile` for append; assign as `cmd.Stdout` and `cmd.Stderr`.
   - `exec.Command(p.Binary, p.Args...)`, env = `os.Environ()` + `p.Env`.
   - `cmd.Start()` — PID assigned.
   - Launch background goroutine that `cmd.Wait()`s and handles restart policy.
   - Poll `p.Ready(ctx)` every 250 ms until success (return `nil`) or `p.ReadyTimeout` elapsed (call `Stop(name, 5s)`, return timeout error).

2. Crash-restart policy:
   - On unexpected exit: log exit code, record exit time.
   - If ≥ 5 restarts in the last 60 s: log "too many crashes, giving up" and stop restarting.
   - Otherwise sleep 2 s, respawn (no Ready-wait on restarts — recovery is best-effort).

3. `Stop(name, timeout)`: disable auto-restart flag, send SIGTERM, wait up to `timeout`, SIGKILL if still alive.

4. `StopAll(timeout)`: `Stop` every process in parallel; caller's `timeout` is per-process, not total.

### `daemon-status.json`

Written by the daemon to `~/.pv/daemon-status.json` on startup and on every supervisor state change:

```json
{
  "pid": 12345,
  "started_at": "2026-04-14T10:30:00Z",
  "supervised": {
    "rustfs": {"pid": 12346, "running": true, "restarts": 0}
  }
}
```

- CLI-only readers. No locking.
- Stale-detection: if `pid` isn't alive, treat file as stale, behave as if daemon is down.

## Data Flow

### `pv service:add s3` (daemon not running)

1. `resolveKind("s3")` returns `kindBinary`, `RustFS{}`, `nil`, `nil`.
2. `binaries.Download(binaries.Rustfs, latest)` fetches `rustfs-{platform}-latest.zip`, extracts to `~/.pv/internal/bin/rustfs`, writes `~/.pv/internal/bin/rustfs.version`.
3. `registry.Put("s3", ServiceInstance{Kind: "binary", Port: 9000, ConsolePort: 9001, Enabled: &trueVal})`.
4. `autoRestart(ctx)` → daemon not running, prints "daemon not running — changes will apply on next `pv start`". No-op otherwise.

### `pv start` (after above)

1. Existing FrankenPHP startup runs.
2. `sup := supervisor.New()`; `defer sup.StopAll(10 * time.Second)`.
3. For each entry in `services.AllBinary()` that is registered and enabled:
   - Translate `BinaryService` → `supervisor.Process` via `buildSupervisorProcess(svc)`.
   - `sup.Start(ctx, proc)`. On failure: log via `ui.Fail`, continue (non-fatal).
4. `writeDaemonStatus(sup)`.
5. Existing signal-handling loop; on shutdown, `sup.StopAll(10 s)` via defer.

### `pv service:stop s3` (daemon running)

1. Load registry; find `s3` entry with `Kind == "binary"`.
2. Set `Enabled = &falseVal`; save registry.
3. `autoRestart(ctx)` → daemon restart. Post-restart, spawn loop skips `s3` because it's disabled. Result: `rustfs` is no longer supervised.
4. Print "s3 disabled; daemon restarted — rustfs no longer supervised."

### `pv service:remove s3` (daemon running)

1. Delete registry entry.
2. Remove `~/.pv/internal/bin/rustfs` and `~/.pv/internal/bin/rustfs.version`.
3. `autoRestart(ctx)` → daemon restart. Supervisor spawn loop skips (no registry entry). Data directory preserved.

### `pv service:logs s3`

Tail `~/.pv/logs/rustfs.log` using the same terminal streaming helper used by the Docker logs path; works whether the daemon is running or not.

## Registry Changes

```go
type ServiceInstance struct {
    Image       string `json:"image,omitempty"`
    Port        int    `json:"port"`
    ConsolePort int    `json:"console_port,omitempty"`
    Kind        string `json:"kind,omitempty"`    // "docker" | "binary"; empty ⇒ "docker"
    Enabled     *bool  `json:"enabled,omitempty"` // binary only; nil ⇒ true
}
```

**Back-compat:** existing entries without `Kind` continue to be treated as Docker. If a pre-upgrade `s3` entry is found (`Kind == ""` but name is in `binaryRegistry`), the first `service:*` invocation silently rewrites it into a binary-shaped entry (`Kind="binary"`, `Enabled=&true`, `Image=""`). One-time migration.

## Command Dispatch

Every `service:*` command starts with:

```go
kind, bin, docker, err := resolveKind(reg, name)
if err != nil { return err }
switch kind {
case kindBinary:
    return <cmd>Binary(ctx, reg, bin, ...)
case kindDocker:
    return <cmd>Docker(ctx, reg, docker, version, ...)
}
```

### Binary command behaviors

| Command | Behavior |
|---------|----------|
| `service:add s3` | Download binary → register (`Kind=binary`, `Enabled=true`) → `autoRestart` |
| `service:start s3` | Error if not registered. Else set `Enabled=true` → `autoRestart` |
| `service:stop s3` | Error if not registered. Else set `Enabled=false` → `autoRestart` |
| `service:remove s3` | Unregister → delete binary → `autoRestart` |
| `service:destroy s3` | `remove` + delete `~/.pv/data/s3/` |
| `service:status s3` | Read `daemon-status.json`; show registered, enabled, pid, restarts |
| `service:list` | Merged table of all services (both kinds) |
| `service:logs s3` | Tail `~/.pv/logs/rustfs.log` |

### `autoRestart` helper

```go
func autoRestart(ctx context.Context) error {
    if !daemon.IsRunning() {
        ui.Subtle("daemon not running — changes will apply on next `pv start`")
        return nil
    }
    return daemon.Restart(ctx) // existing restart path used by `pv restart`
}
```

Called as the final step of every binary-service mutation. Trade-off users should be aware of (to be documented in command help): a binary `service:stop` triggers a brief FrankenPHP restart. Docker services do not pay this cost; only binary services do.

## `pv update` Integration

After the existing tool-update loop in `cmd/update.go`:

```go
for name, svc := range services.AllBinary() {
    if !reg.Has(name) { continue }
    if err := updateBinaryService(ctx, svc); err != nil {
        return fmt.Errorf("update %s: %w", name, err)
    }
}
```

`updateBinaryService` compares the installed version (read from `{binary}.version` sidecar) against `binaries.LatestVersion(svc.Binary())` and re-downloads if newer. If daemon is running and a new binary was written, print "run `pv restart` to run the new version." We do **not** auto-restart here — users running `pv update` may be updating several tools and prefer one restart at the end.

## Non-fatal Startup Failure Policy

If a binary service fails to start during `pv start` (missing binary, port conflict, ReadyCheck timeout), the daemon:

1. Logs the failure via `ui.Fail`.
2. Continues starting any remaining binary services.
3. Completes FrankenPHP startup.
4. Reports the failure in `daemon-status.json` under that service's entry.

Result: a user with a misconfigured `s3` still gets FrankenPHP and their projects; they fix `s3` in a follow-up. This matches the existing non-fatal philosophy elsewhere in the daemon.

## Error Handling

| Failure | Where caught | Behavior |
|---------|-------------|----------|
| Download fails (network, 404) | `service:add`, `pv update` | Return error; registry untouched; binary untouched. User retries. |
| Archive extraction fails | `service:add` | Delete partial file; return error with hint to rerun `pv service:add s3`. |
| Unsupported `GOOS/GOARCH` | `binaries/rustfs.go` | Typed error at URL construction; user sees it before download starts. |
| Port 9000 or 9001 in use | `ReadyCheck` timeout | Supervisor kills spawned process; `sup.Start` returns timeout error; daemon reports and continues. |
| Data dir creation fails (perms) | `service:add` | Return error; registry untouched. |
| Binary missing when daemon spawns | `supervisor.Start` | Returns "binary not found at <path> — run `pv service:add s3`". Daemon continues with other services. |
| Binary crashes on startup | `ReadyCheck` timeout + supervisor | First-start failure → `sup.Start` returns error. **No restart loop on initial-start failure.** |
| Crash loop mid-session (5-in-60s cap) | Supervisor | Log, mark `running: false` in `daemon-status.json`. `service:status` surfaces it. |
| `daemon-status.json` write fails | Daemon | Log warning; continue. Readers fall back to PID check on daemon. |
| `service:start` on unregistered | Command | Error: "s3 not registered — run `pv service:add s3` first". |
| `service:add` on already-registered | Command | Error: "s3 already registered — use `pv service:start s3` to enable". |
| Stale `daemon-status.json` | CLI readers | PID check; if dead, ignore file. |
| Old Docker `s3` registry entry pre-upgrade | Commands | Silent one-time rewrite to binary shape. |

## Edge Cases

- **Upgrade while supervised.** New binary overwrites the on-disk file; running process keeps old binary via its open file descriptor. Take effect on next daemon restart.
- **User manually kills rustfs process.** Supervisor detects as crash, restarts after 2 s. To actually stop: `pv service:stop s3`.
- **Two `pv start` invocations in parallel.** Guarded by existing daemon PID-file singleton logic; inherited.
- **Orphaned children (daemon SIGKILLed).** On next `pv start`, read previous `daemon-status.json`; for each supervised PID still alive, send SIGTERM before spawning fresh. One-time reconciliation at start.

## Testing Strategy

### Unit tests

- `internal/binaries/rustfs_test.go`: URL + archive-name construction for every supported `(GOOS, GOARCH)`; error on unsupported pair.
- `internal/services/rustfs_test.go`: `RustFS{}` method outputs for `Args`, `Env`, `EnvVars`, `WebRoutes`, `ReadyCheck`; `LookupBinary("s3")` finds it, `LookupBinary("mysql")` does not.
- `internal/supervisor/supervisor_test.go`: uses a `//go:build testhelper` helper binary compiled into the test package. Cases: clean spawn + ready, ready-timeout, crash-restart within budget, crash-budget exhaustion, graceful stop with SIGTERM, SIGKILL fallback, parallel `StopAll`.
- `internal/commands/service/*_test.go`: fresh cobra trees + `t.Setenv("HOME", t.TempDir())`. Verify each command mutates registry correctly and calls `autoRestart` as the final step.
- `internal/registry/registry_test.go`: `ServiceInstance` JSON round-trip with / without `Kind` and `Enabled`. Back-compat loading.

### Integration tests

- `internal/supervisor/` in-process integration: spawn helper, verify `daemon-status`-like structs, confirm `StopAll` kills under SIGTERM in <1 s.
- `ReadyCheck` TCP probe exercised against an in-test listener.

### E2E

`scripts/e2e/s3-binary.sh` runs on macOS GitHub Actions:

```bash
pv start
pv service:add s3
# assert rustfs in daemon-status, :9000 responds to HEAD
pv service:stop s3
# assert not in supervisor output
pv service:start s3
# assert back
pv service:destroy s3
# assert gone + data dir removed
pv stop
```

Added as a new phase in `.github/workflows/e2e.yml`.

### Explicitly NOT tested

- Linux binary correctness — CI is macOS-only; Linux is hand-verified at release.
- RustFS on-disk format compatibility across upgrades. Documented limitation: "RustFS is alpha; major version bumps may require `pv service:destroy s3` + re-add."
- Performance / load.

## Verification Items (before implementation starts)

These were assumptions that need ground-truth verification in the first task of the implementation plan:

1. Asset names on RustFS releases for `macos-x86_64`, `linux-x86_64`, `linux-aarch64`. Only `macos-aarch64` is user-confirmed.
2. Exact CLI flags for RustFS — does `./rustfs server --help` accept `--address :9000 --console-address :9001`, or is the syntax different?
3. Archive contents — does the `.zip` have `rustfs` at the root or nested in a subdirectory?
4. Whether RustFS exposes a usable health endpoint (e.g. `/minio/health/live`) — if yes, we can upgrade `ReadyCheck` from TCP to HTTP; if no, TCP is sufficient.

## Deferred (explicit non-goals for this spec, listed for future reference)

- Migration of mail / redis / mysql / postgres to binary services.
- IPC channel between CLI and daemon (replaces `autoRestart` with targeted supervisor reconcile).
- HTTP-based `ReadyCheck` upgrade.
- Log rotation / size caps in supervisor.
- Per-install credential generation.
- Resource limits (CPU, memory) for supervised processes.
- Linux CI coverage.
