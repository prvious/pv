# Server Reconcile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a reconcile-based server manager so that PHP version changes (via pv.yml, pv link, pv unlink) automatically start/stop secondary FrankenPHP instances without requiring daemon restarts.

**Architecture:** A `ServerManager` struct in `internal/server/manager.go` owns all FrankenPHP instances and exposes a `Reconcile()` method that reads configs from disk, diffs running instances against needed instances, and starts/stops accordingly. CLI commands send SIGHUP to the daemon to trigger reconciliation. The daemon's event loop handles SIGHUP alongside SIGINT/SIGTERM.

**Tech Stack:** Go, fsnotify (existing watcher), FrankenPHP/Caddy (existing), Unix signals.

**Spec:** `docs/superpowers/specs/2026-03-27-server-reconcile-design.md`

---

## Task 1: Create ServerManager with Reconcile()

**Files:**
- Create: `internal/server/manager.go`

This is the core. A new file with the `ServerManager` struct, `Reconcile()`, and `Shutdown()`.

- [ ] **Step 1: Create `internal/server/manager.go`**

```go
package server

import (
	"fmt"
	"os"
	"sync"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// ServerManager owns the main and secondary FrankenPHP instances.
// Reconcile() is the single entry point for syncing running instances
// against the current config state on disk.
type ServerManager struct {
	mu          sync.Mutex
	main        *FrankenPHP
	secondaries map[string]*FrankenPHP // version -> instance
}

// NewServerManager creates a manager with the given main FrankenPHP instance.
func NewServerManager(main *FrankenPHP) *ServerManager {
	return &ServerManager{
		main:        main,
		secondaries: make(map[string]*FrankenPHP),
	}
}

// Reconcile reads configs from disk, regenerates Caddyfiles, diffs running
// secondary instances against what's needed, starts missing ones, stops
// unneeded ones, restarts crashed ones, and reloads the main FrankenPHP.
func (m *ServerManager) Reconcile() error {
	m.mu.Lock()
	defer m.mu.Unlock()

	settings, err := config.LoadSettings()
	if err != nil {
		return fmt.Errorf("reconcile: load settings: %w", err)
	}
	globalPHP := settings.Defaults.PHP

	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("reconcile: load registry: %w", err)
	}

	// Regenerate all Caddyfiles from current config state.
	if err := caddy.GenerateAllConfigs(reg.List(), globalPHP); err != nil {
		return fmt.Errorf("reconcile: generate configs: %w", err)
	}

	// Compute which secondary versions are needed.
	needed := caddy.ActiveVersions(reg.List(), globalPHP)

	// Stop secondaries that are no longer needed.
	for version, fp := range m.secondaries {
		if !needed[version] {
			fmt.Fprintf(os.Stderr, "Reconcile: stopping FrankenPHP for PHP %s (no longer needed)\n", version)
			fp.Stop()
			delete(m.secondaries, version)
		}
	}

	// Start missing or crashed secondaries.
	for version := range needed {
		fp, exists := m.secondaries[version]

		// Check if existing instance has crashed.
		if exists {
			select {
			case <-fp.Done():
				// Process exited — remove and re-create.
				fmt.Fprintf(os.Stderr, "Reconcile: FrankenPHP for PHP %s crashed, restarting\n", version)
				delete(m.secondaries, version)
				exists = false
			default:
				// Still running, nothing to do.
			}
		}

		if !exists {
			port := config.PortForVersion(version)
			fmt.Fprintf(os.Stderr, "Reconcile: starting FrankenPHP for PHP %s on port %d\n", version, port)
			newFP, err := StartVersionFrankenPHP(version)
			if err != nil {
				fmt.Fprintf(os.Stderr, "Reconcile: cannot start FrankenPHP for PHP %s: %v\n", version, err)
				continue
			}
			m.secondaries[version] = newFP
		}
	}

	// Reload the main FrankenPHP to pick up new Caddyfile.
	if err := Reload(); err != nil {
		return fmt.Errorf("reconcile: reload main FrankenPHP: %w", err)
	}

	return nil
}

// Shutdown stops all secondary FrankenPHP instances.
// The main instance is stopped separately via its own defer in Start().
func (m *ServerManager) Shutdown() {
	m.mu.Lock()
	defer m.mu.Unlock()

	for version, fp := range m.secondaries {
		fmt.Fprintf(os.Stderr, "Stopping FrankenPHP for PHP %s\n", version)
		fp.Stop()
		delete(m.secondaries, version)
	}
}

// RunningVersions returns the set of PHP versions with active secondary instances.
// Used for testing and diagnostics.
func (m *ServerManager) RunningVersions() []string {
	m.mu.Lock()
	defer m.mu.Unlock()

	var versions []string
	for v := range m.secondaries {
		versions = append(versions, v)
	}
	return versions
}
```

- [ ] **Step 2: Build and verify**

```bash
go build ./...
go vet ./...
```

- [ ] **Step 3: Commit**

```bash
git add internal/server/manager.go
git commit -m "Add ServerManager with Reconcile for secondary FrankenPHP lifecycle

ServerManager owns all FrankenPHP instances and provides a single
Reconcile() method that reads configs from disk, diffs running
instances against what's needed, and starts/stops accordingly.
Handles crashed instance detection and restart."
```

---

## Task 2: Add SignalDaemon() and SIGHUP handler

**Files:**
- Modify: `internal/server/process.go`

Add `SignalDaemon()` for CLI commands and refactor the event loop to handle SIGHUP.

- [ ] **Step 1: Add `SignalDaemon()` function**

Add to `internal/server/process.go`:

```go
// SignalDaemon sends SIGHUP to the running daemon process, triggering a
// reconciliation of FrankenPHP instances. Safe to call when daemon is not
// running (returns nil).
func SignalDaemon() error {
	pid, err := ReadPID()
	if err != nil {
		return nil // daemon not running
	}
	proc, err := os.FindProcess(pid)
	if err != nil {
		return nil
	}
	// Signal 0 first to check if process exists.
	if proc.Signal(syscall.Signal(0)) != nil {
		return nil // process doesn't exist
	}
	return proc.Signal(syscall.SIGHUP)
}
```

- [ ] **Step 2: Build and verify**

```bash
go build ./...
go vet ./...
```

- [ ] **Step 3: Commit**

```bash
git add internal/server/process.go
git commit -m "Add SignalDaemon() to send SIGHUP to running daemon"
```

---

## Task 3: Refactor Start() to use ServerManager

**Files:**
- Modify: `internal/server/process.go`

This is the biggest change. Replace inline secondary management with the manager, add SIGHUP to the event loop, and wire the watcher to call `Reconcile()`.

- [ ] **Step 1: Add package-level manager variable**

Add near the top of `process.go`, next to `activeWatcher`:

```go
// manager holds the server manager for FrankenPHP instances.
// Set during Start(), used by the watcher and SIGHUP handler.
var manager *ServerManager
```

- [ ] **Step 2: Refactor `Start()` to use manager**

Replace the inline secondary startup (lines 89-107) and the `waitForEvent` call with:

1. After starting main FrankenPHP, create the manager:
   ```go
   manager = NewServerManager(mainFP)
   defer manager.Shutdown()
   ```

2. Call `Reconcile()` instead of inline secondary startup:
   ```go
   if err := manager.Reconcile(); err != nil {
       fmt.Fprintf(os.Stderr, "Warning: initial reconcile: %v\n", err)
   }
   ```

3. Remove the local `secondaries` variable and the `defer` that stops them.

4. Remove the `caddy.GenerateAllConfigs()` call at the top of `Start()` — `Reconcile()` handles it now.

5. Add SIGHUP to signal handler:
   ```go
   signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
   ```

- [ ] **Step 3: Simplify `waitForEvent()`**

Replace the current `waitForEvent` that watches a static secondaries slice. The new version only watches main FrankenPHP, DNS, and signals:

```go
func waitForEvent(sigCh chan os.Signal, dnsErr chan error, mainFP *FrankenPHP) error {
	for {
		select {
		case sig := <-sigCh:
			if sig == syscall.SIGHUP {
				fmt.Fprintf(os.Stderr, "Received SIGHUP, reconciling...\n")
				if err := manager.Reconcile(); err != nil {
					fmt.Fprintf(os.Stderr, "Warning: reconcile failed: %v\n", err)
				}
				continue
			}
			fmt.Fprintf(os.Stderr, "\nReceived %s, shutting down...\n", sig)
			return nil
		case err := <-dnsErr:
			if err != nil {
				return fmt.Errorf("DNS server failed: %w", err)
			}
			return fmt.Errorf("DNS server exited unexpectedly")
		case err := <-mainFP.Done():
			if err != nil {
				return fmt.Errorf("FrankenPHP exited unexpectedly: %w", err)
			}
			return fmt.Errorf("FrankenPHP exited unexpectedly")
		}
	}
}
```

- [ ] **Step 4: Update `handleWatcherEvents()` to call `Reconcile()`**

Replace `ReconfigureServer()` call on line 298 with `manager.Reconcile()`.

- [ ] **Step 5: Remove `ReconfigureServer()`**

Delete the `ReconfigureServer()` function entirely (lines 193-213). It's replaced by `manager.Reconcile()` inside the daemon and `SignalDaemon()` from CLI.

- [ ] **Step 6: Build and run all tests**

```bash
go build ./...
go vet ./...
go test ./...
```

- [ ] **Step 7: Commit**

```bash
git add internal/server/process.go
git commit -m "Refactor Start() to use ServerManager with SIGHUP reconcile

Replace inline secondary management with ServerManager.Reconcile().
Add SIGHUP handler to event loop — CLI commands send SIGHUP to
trigger reconciliation. Remove ReconfigureServer() — replaced by
Reconcile() inside daemon and SignalDaemon() from CLI."
```

---

## Task 4: Update CLI commands to use SignalDaemon()

**Files:**
- Modify: `cmd/link.go`
- Modify: `cmd/unlink.go`
- Modify: `cmd/restart.go`
- Modify: `internal/commands/php/use.go`

- [ ] **Step 1: Simplify `cmd/link.go`**

Replace lines 143-158 (the restart/reconfigure dance) with:

```go
// Signal the daemon to reconcile FrankenPHP instances.
if server.IsRunning() {
    if err := server.SignalDaemon(); err != nil {
        ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
    }
}
```

Remove the `daemon` import and `needsRestart` logic — no longer needed.

- [ ] **Step 2: Simplify `cmd/unlink.go`**

Replace lines 94-117 (the orphan-detection + restart dance) with:

```go
// Signal the daemon to reconcile — it will stop orphaned secondaries.
if server.IsRunning() {
    if err := server.SignalDaemon(); err != nil {
        ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
    }
}
```

Remove the orphan-detection logic, `caddy.ActiveVersions` import, `globalPHP` lookup for the orphan check, etc.

- [ ] **Step 3: Simplify `cmd/restart.go`**

Replace the dual foreground/daemon logic with: always do a full restart.

```go
RunE: func(cmd *cobra.Command, args []string) error {
    if daemon.IsLoaded() {
        return daemoncmds.RunRestart()
    }

    if !server.IsRunning() {
        return fmt.Errorf("pv is not running")
    }

    return ui.Step("Restarting server...", func() (string, error) {
        // Foreground mode: send SIGHUP for a reconcile.
        // For a true restart, user should pv stop && pv start.
        if err := server.SignalDaemon(); err != nil {
            return "", fmt.Errorf("cannot signal server: %w", err)
        }
        return "Server configuration reconciled", nil
    })
},
```

Remove the `server.ReconfigureServer` import reference.

- [ ] **Step 4: Simplify `internal/commands/php/use.go`**

Replace lines 49-60 with: always full daemon restart since the main binary changes.

```go
// The global PHP binary changed — daemon needs full restart.
if oldV != version && server.IsRunning() {
    if daemon.IsLoaded() {
        if err := daemon.Restart(); err != nil {
            ui.Fail(fmt.Sprintf("Could not restart daemon: %v — run 'pv restart' manually", err))
        } else {
            ui.Success("Daemon restarted with new PHP version")
        }
    } else {
        ui.Subtle("Server is running in foreground — restart required.")
        ui.Subtle("Run: pv stop && pv start")
    }
}
```

Remove the `daemon.SyncIfNeeded` / plist sync logic.

- [ ] **Step 5: Build and run all tests**

```bash
go build ./...
go vet ./...
go test ./...
```

- [ ] **Step 6: Commit**

```bash
git add cmd/link.go cmd/unlink.go cmd/restart.go internal/commands/php/use.go
git commit -m "Simplify CLI commands to use SignalDaemon() for reconcile

- pv link: 15 lines of restart logic → SignalDaemon()
- pv unlink: 23 lines of orphan detection → SignalDaemon()
- pv restart: always full restart in daemon mode, SIGHUP in foreground
- pv php:use: always full daemon restart (main binary changes)"
```

---

## Task 5: Add service commands SignalDaemon() for Caddy reload

**Files:**
- Modify: `internal/commands/service/add.go`
- Modify: `internal/commands/service/remove.go`
- Modify: `internal/commands/service/destroy.go`

Service commands generate service site configs but currently never tell FrankenPHP to reload. The daemon needs to pick up the new routes.

- [ ] **Step 1: Add SignalDaemon() after service config generation**

In each of the three files, after the `caddy.GenerateServiceSiteConfigs(reg)` call, add:

```go
// Signal daemon to reload and pick up new service routes.
if server.IsRunning() {
    _ = server.SignalDaemon()
}
```

Add import: `"github.com/prvious/pv/internal/server"`

- [ ] **Step 2: Build and verify**

```bash
go build ./...
go vet ./...
go test ./...
```

- [ ] **Step 3: Commit**

```bash
git add internal/commands/service/add.go internal/commands/service/remove.go internal/commands/service/destroy.go
git commit -m "Signal daemon after service config changes for Caddy reload

Service add/remove/destroy generate service site configs but never
told FrankenPHP to reload. Now send SIGHUP so the daemon reconciles
and the main FrankenPHP picks up new service routes."
```

---

## Parallelization Guide

**Task 1** must come first (creates manager.go).

**Task 2** must come after Task 1 (adds SignalDaemon, depends on same file).

**Task 3** must come after Task 2 (refactors Start to use manager).

**Tasks 4 and 5** can run in parallel after Task 3 (they touch different files).

```
Task 1 → Task 2 → Task 3 → Task 4 (parallel)
                           → Task 5 (parallel)
```
