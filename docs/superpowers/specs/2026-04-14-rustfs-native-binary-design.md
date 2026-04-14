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
- Do **not** build new CLI↔daemon IPC. `server.SignalDaemon()` (existing SIGHUP mechanism, see `docs/superpowers/specs/2026-03-27-server-reconcile-design.md`) is the channel for state changes.
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
| `scripts/e2e/s3-binary.sh` | New e2e phase that exercises the full binary flow |

### Modified files

| Path | Change |
|------|--------|
| `internal/services/service.go` | Narrow `registry` (Docker-only) to mail/mysql/postgres/redis; add `LookupDocker`, `Lookup(kind)`; `Available()` returns union |
| `internal/binaries/manager.go` | Add `"rustfs":` cases in `DownloadURL` and `LatestVersionURL`. Do **not** add Rustfs to `Tools()` (it's a backing service, not a user-exposed tool) |
| `internal/registry/registry.go` | `ServiceInstance` gains `Kind string` and `Enabled *bool` fields (both JSON `omitempty`; missing defaults preserve current Docker behavior) |
| `internal/server/manager.go` | `ServerManager` gains a `supervisor *supervisor.Supervisor` field. `Reconcile()` is extended with a binary-service reconcile phase (diff registry vs. supervisor state, start/stop as needed). `Shutdown()` calls `supervisor.StopAll(10 * time.Second)` |
| `internal/server/process.go` | `Start()` instantiates the supervisor, hands it to `NewServerManager`, and writes `daemon-status.json` from the reconcile path. Existing SIGHUP handler unchanged — it calls `Reconcile()` which now also handles binary services |
| `internal/commands/service/add.go` | Use `resolveKind`; binary path downloads + registers + `server.SignalDaemon()` |
| `internal/commands/service/start.go` | Binary path: set `Enabled=true`, `server.SignalDaemon()` |
| `internal/commands/service/stop.go` | Binary path: set `Enabled=false`, `server.SignalDaemon()` |
| `internal/commands/service/remove.go` | Binary path: unregister + delete binary + `server.SignalDaemon()` |
| `internal/commands/service/destroy.go` | Binary path: `remove` + delete `config.ServiceDataDir("s3", "latest")` (resolves to `~/.pv/services/s3/latest/data`) |
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
- `Args(dataDir)`: `["server", dataDir, "--address", ":9000", "--console-enable", "--console-address", ":9001"]`
  - **Verified 2026-04-14** against `rustfs 1.0.0-alpha.93`. The positional `<VOLUMES>` is the data-dir. `--console-enable` is required to actually open port 9001; without it, the console UI never binds.
- `Env()`: `["RUSTFS_ACCESS_KEY=rstfsadmin", "RUSTFS_SECRET_KEY=rstfsadmin"]`
  - **Verified 2026-04-14**: the `RUSTFS_ROOT_USER` / `RUSTFS_ROOT_PASSWORD` names in earlier drafts are invalid; the real env vars are `RUSTFS_ACCESS_KEY` and `RUSTFS_SECRET_KEY` (per `rustfs server --help`).
- `Port()`: `9000`; `ConsolePort()`: `9001`
- `WebRoutes()`: `[{s3 → 9001}, {s3-api → 9000}]` (unchanged from current Docker version)
- `EnvVars(project)`: identical keys/values to the current Docker S3 so linked projects keep working
- `ReadyCheck()`: `TCPPort: 9000, Timeout: 30s`

### `binaries.Rustfs` (`internal/binaries/rustfs.go`)

- Archive naming: `rustfs-{platform}-latest.zip`, where `{platform}` is (verified 2026-04-14 against alpha.93):
  - `darwin/arm64` → `macos-aarch64`
  - `darwin/amd64` → `macos-x86_64`
  - `linux/amd64` → `linux-x86_64-gnu` (RustFS publishes `-gnu` and `-musl` variants on Linux; pv uses the glibc build)
  - `linux/arm64` → `linux-aarch64-gnu`
- Download URL pattern: `https://github.com/rustfs/rustfs/releases/download/{version}/rustfs-{platform}-latest.zip`
- Latest version: `https://api.github.com/repos/rustfs/rustfs/releases?per_page=1` — the `/releases/latest` endpoint returns 404 because RustFS marks every release as a prerelease. `FetchLatestVersion` must special-case rustfs to parse an array response and take `[0].tag_name`.
- `NeedsExtract: true` — `.zip` archive, extract to `~/.pv/internal/bin/rustfs`. The `rustfs` binary sits at the **root** of the zip (verified 2026-04-14).

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

### `ServerManager` extension (existing type, new responsibility)

`ServerManager` already owns the main + secondary FrankenPHP instances and exposes `Reconcile()` for SIGHUP-driven reconciliation (see `docs/superpowers/specs/2026-03-27-server-reconcile-design.md`). We extend it:

```go
type ServerManager struct {
    mu          sync.Mutex
    main        *FrankenPHP
    secondaries map[string]*FrankenPHP
    supervisor  *supervisor.Supervisor // NEW
}
```

`Reconcile()` gains a second phase after the existing FrankenPHP phase:

```go
func (m *ServerManager) Reconcile() error {
    m.mu.Lock()
    defer m.mu.Unlock()

    // (existing) regenerate Caddyfiles, diff secondaries, reload main

    // NEW: reconcile binary services
    if err := m.reconcileBinaryServices(); err != nil {
        // non-fatal — log and continue
        fmt.Fprintf(os.Stderr, "Reconcile: binary service(s) failed: %v\n", err)
    }

    // NEW: write daemon-status.json reflecting supervisor state
    writeDaemonStatus(m.supervisor)

    return nil
}

func (m *ServerManager) reconcileBinaryServices() error {
    reg, err := registry.Load()
    if err != nil { return err }

    needed := map[string]services.BinaryService{} // enabled + registered
    for name, svc := range services.AllBinary() {
        entry := reg.FindService(name)
        if entry == nil || entry.Kind != "binary" { continue }
        if entry.Enabled != nil && !*entry.Enabled { continue }
        needed[name] = svc
    }

    // Stop supervised processes that are no longer needed.
    for _, name := range m.supervisor.SupervisedNames() {
        if _, ok := needed[name]; !ok {
            m.supervisor.Stop(name, 10*time.Second)
        }
    }

    // Start processes that are needed but not running.
    for name, svc := range needed {
        if m.supervisor.IsRunning(name) { continue }
        proc, err := buildSupervisorProcess(svc)
        if err != nil {
            // collect error, continue with the rest
            continue
        }
        if err := m.supervisor.Start(ctx, proc); err != nil {
            // collect error, continue
            continue
        }
    }
    return nil
}
```

`Shutdown()` gets one line added: `m.supervisor.StopAll(10 * time.Second)`.

`SupervisedNames()` is a small new method on `Supervisor` returning the keys of its internal processes map — existing supervisor API from §3 wasn't quite enough.

`buildSupervisorProcess(svc BinaryService) (supervisor.Process, error)` is a helper in `internal/server/` that translates a `BinaryService` into a `supervisor.Process`. It resolves paths via `internal/config`:

- `Binary`: `filepath.Join(config.InternalBinDir(), svc.Binary().Name)` → `~/.pv/internal/bin/rustfs`
- `dataDir`: `config.ServiceDataDir(svc.Name(), "latest")` → `~/.pv/services/s3/latest/data`. Created with `os.MkdirAll(dataDir, 0o755)` before returning.
- `LogFile`: `filepath.Join(config.PvDir(), "logs", svc.Binary().Name+".log")` → `~/.pv/logs/rustfs.log`. Parent directory created if missing.
- `Args`: `svc.Args(dataDir)`
- `Env`: `svc.Env()`
- `Ready`: constructs a closure from `svc.ReadyCheck()` (TCP-dial or HTTP-GET per the type)
- `ReadyTimeout`: from `svc.ReadyCheck().Timeout`

This helper is the single place that translates between the `BinaryService` contract and `supervisor.Process` — no other code should construct a `Process` for a service directly.

### `daemon-status.json`

Written by the daemon from `Reconcile()` — so every SIGHUP produces a fresh snapshot:

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
- On crash-budget exhaustion inside the supervisor, the supervisor marks the process as not running; next reconcile (the 5-in-60s cap is a background event, not a reconcile trigger) will reflect that in the JSON. For immediate freshness after a background event, we can add a callback from supervisor → ServerManager later; v1 keeps it pull-based for simplicity.

## Data Flow

### `pv service:add s3` (daemon not running)

1. `resolveKind("s3")` returns `kindBinary`, `RustFS{}`, `nil`, `nil`.
2. `binaries.Download(binaries.Rustfs, latest)` fetches `rustfs-{platform}-latest.zip`, extracts to `~/.pv/internal/bin/rustfs`, writes `~/.pv/internal/bin/rustfs.version`.
3. `registry.Put("s3", ServiceInstance{Kind: "binary", Port: 9000, ConsolePort: 9001, Enabled: &trueVal})`.
4. `server.SignalDaemon()` is a no-op (PID file missing or stale). Print "daemon not running — changes will apply on next `pv start`".

### `pv start` (after above)

1. Existing FrankenPHP startup runs.
2. `sup := supervisor.New()`; pass it to `NewServerManager(main, sup)`.
3. `manager.Reconcile()` runs as part of boot (existing behavior). The new binary-service phase observes the registry entry for `s3`, sees nothing supervised yet, and calls `sup.Start(ctx, proc)`. On per-service failure: log and continue (non-fatal).
4. `writeDaemonStatus(sup)` runs from inside `Reconcile()`.
5. Existing signal-handling loop. On shutdown, `manager.Shutdown()` calls `sup.StopAll(10s)`.

### `pv service:stop s3` (daemon running)

1. Load registry; find `s3` entry with `Kind == "binary"`.
2. Set `Enabled = &falseVal`; save registry.
3. `server.SignalDaemon()` → daemon receives SIGHUP → `Reconcile()` runs. Binary-service phase sees `s3` in `needed={}` (disabled), calls `supervisor.Stop("rustfs", 10s)`. FrankenPHP is untouched.
4. Print "s3 disabled".

### `pv service:remove s3` (daemon running)

1. Delete registry entry.
2. Remove `~/.pv/internal/bin/rustfs` and `~/.pv/internal/bin/rustfs.version`.
3. `server.SignalDaemon()` → `Reconcile()` → supervisor sees `s3` is no longer needed → `Stop("rustfs", 10s)`. Data directory preserved.
4. Print "s3 unregistered, rustfs removed".

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

**Back-compat:** existing entries without `Kind` continue to be treated as Docker. **No auto-migration** is performed for a pre-existing Docker-shaped `s3` entry — if a user has one, running `service:add s3` errors out with "s3 already registered (as docker)". The documented remedy is `pv uninstall && pv setup`; pv is still young enough that this is acceptable, and auto-migration adds complexity we don't need to absorb here.

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
| `service:add s3` | Download binary → register (`Kind=binary`, `Enabled=true`) → `server.SignalDaemon()` |
| `service:start s3` | Error if not registered. Else set `Enabled=true` → `server.SignalDaemon()` |
| `service:stop s3` | Error if not registered. Else set `Enabled=false` → `server.SignalDaemon()` |
| `service:remove s3` | Unregister → delete binary → `server.SignalDaemon()` |
| `service:destroy s3` | `remove` + delete `config.ServiceDataDir("s3", "latest")` |
| `service:status s3` | Read `daemon-status.json`; show registered, enabled, pid, restarts |
| `service:list` | Merged table of all services (both kinds) |
| `service:logs s3` | Tail `~/.pv/logs/rustfs.log` |

### SignalDaemon usage

Every binary-service mutation ends with `server.SignalDaemon()` (already exported from `internal/server/process.go`, already used by `cmd/link.go`, `cmd/unlink.go`, `cmd/restart.go`). If the daemon isn't running, `SignalDaemon` is a no-op — the mutation is persisted, and the state change takes effect on next `pv start`. If the daemon is running, SIGHUP triggers `Reconcile()` which adjusts supervisor state in place — **no FrankenPHP restart, no dropped connections**. This is strictly better than a full daemon restart for this use case.

The CLI command prints a terse confirmation after `SignalDaemon()` returns:

```
$ pv service:stop s3
✓ s3 disabled
```

When the daemon isn't running, the message is explicit:

```
$ pv service:add s3
✓ Downloaded rustfs 1.0.0-alpha.93
✓ Registered s3 (binary, enabled)
  daemon not running — service will start on next `pv start`
```

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

`updateBinaryService` compares the installed version (read from `{binary}.version` sidecar) against `binaries.LatestVersion(svc.Binary())` and re-downloads if newer. Running supervisor instances keep the *old* binary open via their file descriptor (standard Unix behavior) — a plain `Reconcile()` won't swap the process because `IsRunning()` still reports true. To load the new binary the user runs either:

- `pv service:stop s3 && pv service:start s3` — cycles just that supervised process, FrankenPHP untouched (preferred), or
- `pv restart` — full daemon restart

After the update loop finishes, `pv update` prints a single hint if any binaries were replaced:

```
Updated binaries: rustfs. Run `pv service:stop s3 && pv service:start s3` (or `pv restart`) to load them.
```

We do **not** auto-cycle here — users running `pv update` may be updating several things and prefer to sequence restarts themselves.

## Non-fatal Reconcile Policy

Whether it's boot-time reconcile or SIGHUP-driven reconcile, a failure to start or stop any single binary service (missing binary, port conflict, ReadyCheck timeout, SIGTERM ignored) does **not** abort reconciliation. The pattern:

1. `reconcileBinaryServices()` collects per-service errors instead of returning on first failure.
2. The outer `Reconcile()` logs collected errors via `fmt.Fprintf(os.Stderr, ...)` and returns nil so the FrankenPHP side of reconciliation isn't disrupted.
3. `daemon-status.json` reflects the failure under that service's entry (next reconcile writes a fresh snapshot).

This matches the existing behavior for FrankenPHP secondary failures (`startErrors` aggregation in `manager.go`). Result: a user with a misconfigured `s3` still gets FrankenPHP and their projects; they fix `s3` in a follow-up and `SignalDaemon()` to retry.

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
| Pre-existing Docker-shaped `s3` entry | `service:add s3` | Error: "s3 already registered (as docker). Run `pv uninstall && pv setup` to reset." No auto-migration. |

## Edge Cases

- **Upgrade while supervised.** New binary overwrites the on-disk file; running process keeps old binary via its open file descriptor. Take effect on `pv service:stop s3 && pv service:start s3` or full daemon restart.
- **User manually kills rustfs process.** Supervisor detects as crash, restarts after 2 s. To actually stop: `pv service:stop s3`.
- **Two `pv start` invocations in parallel.** Guarded by existing daemon PID-file singleton logic; inherited.
- **Orphaned children (daemon SIGKILLed).** On next `pv start`, read previous `daemon-status.json`; for each supervised PID still alive, send SIGTERM before spawning fresh. One-time reconciliation at start.

## Testing Strategy

### Unit tests

- `internal/binaries/rustfs_test.go`: URL + archive-name construction for every supported `(GOOS, GOARCH)`; error on unsupported pair.
- `internal/services/rustfs_test.go`: `RustFS{}` method outputs for `Args`, `Env`, `EnvVars`, `WebRoutes`, `ReadyCheck`; `LookupBinary("s3")` finds it, `LookupBinary("mysql")` does not.
- `internal/supervisor/supervisor_test.go`: uses a `//go:build testhelper` helper binary compiled into the test package. Cases: clean spawn + ready, ready-timeout, crash-restart within budget, crash-budget exhaustion, graceful stop with SIGTERM, SIGKILL fallback, parallel `StopAll`.
- `internal/commands/service/*_test.go`: fresh cobra trees + `t.Setenv("HOME", t.TempDir())`. Verify each command mutates registry correctly and calls `server.SignalDaemon()` as the final step (inject a test double via a package-level var so the test doesn't actually send signals).
- `internal/server/manager_test.go`: extend existing tests. New cases: `Reconcile()` with an empty registry produces no supervisor calls; `Reconcile()` with a binary entry calls `supervisor.Start`; `Reconcile()` after `Enabled=false` calls `supervisor.Stop`; non-fatal behavior when `supervisor.Start` errors.
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
# assert: registry no longer contains "s3"
# assert: `~/.pv/internal/bin/rustfs` is gone
# assert: `~/.pv/services/s3/latest/data` is gone
pv stop
```

Added as a new phase in `.github/workflows/e2e.yml`.

### Explicitly NOT tested

- Linux binary correctness — CI is macOS-only; Linux is hand-verified at release.
- RustFS on-disk format compatibility across upgrades. Documented limitation: "RustFS is alpha; major version bumps may require `pv service:destroy s3` + re-add."
- Performance / load.

## Verification Items (verified 2026-04-14 against alpha.93)

1. Asset names on RustFS releases: confirmed. macOS uses `macos-{aarch64,x86_64}`; Linux uses `linux-{aarch64,x86_64}-{gnu,musl}`. pv ships the `-gnu` variant.
2. CLI flags: confirmed. `rustfs server <VOLUMES>... [--address :9000] [--console-enable] [--console-address :9001]`. `<VOLUMES>` is a positional argument, and `--console-enable` is required to bind port 9001.
3. Archive contents: the `rustfs` binary is at the root of the `.zip` (no subdirectory).
4. Health endpoint: not investigated; `ReadyCheck` stays TCP-based per the original decision.
5. Latest-version API: RustFS marks every release as prerelease, so `/releases/latest` 404s. `FetchLatestVersion` for rustfs uses `/releases?per_page=1` and parses `[0].tag_name` instead.

## Deferred (explicit non-goals for this spec, listed for future reference)

- Migration of mail / redis / mysql / postgres to binary services.
- Richer CLI↔daemon IPC beyond SIGHUP (e.g. a Unix socket with typed responses so the CLI can confirm reconcile succeeded before returning).
- HTTP-based `ReadyCheck` upgrade.
- Log rotation / size caps in supervisor.
- Per-install credential generation.
- Resource limits (CPU, memory) for supervised processes.
- Linux CI coverage.
