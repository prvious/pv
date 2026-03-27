# Server Reconcile Architecture

## Problem

When a `pv.yml` file changes to a non-global PHP version, the watcher updates the registry and regenerates Caddyfiles, but no secondary FrankenPHP instance is started for the new version — resulting in a 502 error. Secondary instances are only started at daemon boot and never managed afterward. The same problem affects `pv link` with a non-global PHP version.

## Core Insight: Two Process Boundaries

The CLI process (`pv link`, `pv unlink`, etc.) and the daemon are separate OS processes. The CLI owns user interaction, config generation, and disk writes. The daemon owns running FrankenPHP instances. They communicate through:

1. **Shared disk state** — registry.json, Caddyfiles, settings (configs are the source of truth)
2. **SIGHUP signal** — CLI tells daemon "configs changed, reconcile yourself"
3. **FrankenPHP admin API** — `frankenphp reload` (already used today)

## Design

### New: `ServerManager` (inside daemon process)

A package-level struct in `internal/server/` that owns all FrankenPHP instances:

```go
type ServerManager struct {
    mu          sync.Mutex
    main        *FrankenPHP
    secondaries map[string]*FrankenPHP // version -> instance
}
```

### `Reconcile()` — the daemon's single reconciliation function

Called internally by the daemon when SIGHUP is received or on boot. NOT called from CLI processes.

Steps:
1. Load settings + registry from disk (source of truth)
2. Regenerate all Caddyfiles from current state
3. Compute needed secondary versions via `caddy.ActiveVersions()`
4. Diff against running secondaries:
   - **Missing versions** → `StartVersionFrankenPHP(version)`, add to map
   - **Unneeded versions** → `fp.Stop()`, remove from map
   - **Crashed instances** (in map but process dead) → restart
5. Reload main FrankenPHP (picks up new Caddyfile)

### SIGHUP Handler

The daemon's `Start()` adds SIGHUP to its signal listener. On SIGHUP, it calls `Reconcile()`:

```go
signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)

// In event loop:
case sig := <-sigCh:
    if sig == syscall.SIGHUP {
        manager.Reconcile()
        continue  // don't exit, keep running
    }
    // SIGINT/SIGTERM → shutdown
```

### Helper: `SignalDaemon()`

A new exported function that CLI commands call to send SIGHUP to the running daemon:

```go
func SignalDaemon() error {
    pid, err := ReadPID()
    if err != nil {
        return nil // daemon not running, nothing to signal
    }
    proc, err := os.FindProcess(pid)
    if err != nil {
        return nil
    }
    return proc.Signal(syscall.SIGHUP)
}
```

## Who triggers what

| Trigger | What happens | Why |
|---------|-------------|-----|
| **`pv link`** | CLI: pipeline + save configs to disk. Then `SignalDaemon()` (SIGHUP). | Daemon reconciles — starts secondary if new PHP version needed. |
| **`pv unlink`** | CLI: remove site config, save registry. Then `SignalDaemon()`. | Daemon reconciles — stops orphaned secondary if no projects use that version. |
| **`pv restart`** | Full daemon restart (`launchctl kickstart -k` or stop+start). | Clean slate — main binary reloaded, all secondaries rebuilt from configs. |
| **`pv php:use`** | CLI: update settings. Full daemon restart. | Main FrankenPHP binary changes — need process restart, not just config reload. |
| **Watcher** (pv.yml change) | Direct `Reconcile()` inside daemon. | Already in daemon process — no signal needed. |
| **Daemon boot** | `Reconcile()` after main FrankenPHP starts. | Initial secondary startup from config state. |
| **`pv service:*`** | CLI: `caddy.GenerateServiceSiteConfigs()`. Then `SignalDaemon()`. | Service routes need main FrankenPHP reload. No secondary changes. |

## Changes to existing commands

### `cmd/link.go` (lines 143-158)

Before:
```go
if server.IsRunning() {
    needsRestart := phpVersion != "" && phpVersion != globalPHP
    if needsRestart && daemon.IsLoaded() {
        daemon.Restart()
    } else {
        server.ReconfigureServer()
        if needsRestart {
            ui.Subtle("restart required...")
        }
    }
}
```

After:
```go
if server.IsRunning() {
    server.SignalDaemon()
}
```

### `cmd/unlink.go` (lines 94-117)

Before: Complex orphan detection + conditional restart/reconfigure.

After:
```go
if server.IsRunning() {
    server.SignalDaemon()
}
```

### `cmd/restart.go`

Before: Foreground mode calls `ReconfigureServer()`. Daemon mode calls `daemon:restart`.

After: Always full restart — `daemon.Restart()` if daemon mode, `stop+start` if foreground.

### `internal/commands/php/use.go` (lines 49-60)

Before: Sync plist + restart daemon, or tell user to restart.

After: Always full daemon restart (main binary changed).

### `internal/server/process.go` — watcher handler

Before: `ReconfigureServer()` (only reloads config, no secondary management).

After: `manager.Reconcile()` (reloads config AND manages secondaries).

### `internal/server/process.go` — `Start()`

Before: Inline secondary startup with local `secondaries` variable.

After:
```
Start():
  write PID
  load configs
  start DNS
  start main FrankenPHP → store in manager.main
  manager.Reconcile()    → generates Caddyfiles, starts needed secondaries
  start watcher          → on change: update registry, Reconcile()
  register SIGHUP        → on signal: Reconcile()
  start Colima (background)
  start package updater
  event loop (SIGINT/SIGTERM → shutdown, SIGHUP → reconcile)
```

### `ReconfigureServer()` — removed

Replaced by `Reconcile()` inside the daemon and `SignalDaemon()` from CLI.

## Changes to `waitForEvent()`

Currently watches a static slice of secondaries for crashes. With the manager:
- Main FrankenPHP crash → fatal, daemon exits (same as today)
- Secondary crash → detected by `Reconcile()` on next trigger (watcher, SIGHUP, or we add a goroutine that watches the Done() channels and triggers Reconcile)
- DNS error → fatal, daemon exits (same as today)

## Files changed

| File | Change |
|------|--------|
| `internal/server/manager.go` | NEW — `ServerManager`, `Reconcile()`, `Shutdown()` |
| `internal/server/process.go` | Refactor `Start()` to use manager. Add SIGHUP handler. Add `SignalDaemon()`. Remove `ReconfigureServer()`. Simplify `waitForEvent()`. |
| `cmd/link.go` | Replace restart/reconfigure dance with `SignalDaemon()` |
| `cmd/unlink.go` | Replace orphan-detection + restart dance with `SignalDaemon()` |
| `cmd/restart.go` | Always full restart |
| `internal/commands/php/use.go` | Always full daemon restart |
| `internal/server/frankenphp.go` | No changes — `StartVersionFrankenPHP()` stays as-is |

## What does NOT change

- DNS server lifecycle (started once in Start, stopped on shutdown)
- Main FrankenPHP startup (started once in Start)
- Watcher logic for detecting pv.yml changes
- Colima boot logic
- `caddy.GenerateAllConfigs()` / `caddy.ActiveVersions()`
- Service commands (only touch service Caddy configs)
- Automation pipeline in `pv link` (still runs in CLI process)
