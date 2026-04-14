package server

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/packages"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/supervisor"
	"github.com/prvious/pv/internal/watcher"
)

// activeWatcher holds the file watcher for pv.yml changes in linked projects.
var activeWatcher *watcher.Watcher

// manager holds the ServerManager for FrankenPHP instances.
// Set during Start(), used by the watcher and SIGHUP handler.
var manager *ServerManager

// Start is the supervisor entry point. It writes a PID file, starts the DNS
// server, the main FrankenPHP, and any needed secondary FrankenPHP instances,
// then blocks until an OS signal or child exit.
func Start(tld string) error {
	if err := config.EnsureDirs(); err != nil {
		return fmt.Errorf("cannot create directories: %w", err)
	}

	if err := writePID(); err != nil {
		return fmt.Errorf("cannot write PID file: %w", err)
	}
	defer removePID()

	settings, err := config.LoadSettings()
	if err != nil {
		return fmt.Errorf("cannot load settings: %w", err)
	}

	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}

	// Generate initial Caddyfiles so main FrankenPHP can start.
	if err := caddy.GenerateAllConfigs(reg.List(), settings.Defaults.PHP); err != nil {
		return fmt.Errorf("cannot generate caddy configs: %w", err)
	}

	// Start DNS server in a goroutine.
	dnsServer := NewDNSServer(tld)
	dnsErr := make(chan error, 1)
	go func() { dnsErr <- dnsServer.Start() }()
	defer func() {
		if err := dnsServer.Shutdown(); err != nil {
			fmt.Fprintf(os.Stderr, "Warning: DNS server shutdown error: %v\n", err)
		}
	}()

	// Wait for DNS server to bind before proceeding.
	select {
	case <-dnsServer.Ready():
		// Port bound successfully.
	case err := <-dnsErr:
		return fmt.Errorf("DNS server failed to start: %w", err)
	case <-time.After(5 * time.Second):
		return fmt.Errorf("DNS server did not start within 5s")
	}

	fmt.Fprintf(os.Stderr, "DNS server listening on %s\n", dnsServer.Addr)

	// Start main FrankenPHP.
	mainFP, err := StartFrankenPHP()
	if err != nil {
		return fmt.Errorf("cannot start FrankenPHP: %w", err)
	}
	defer mainFP.Stop()

	fmt.Fprintln(os.Stderr, "FrankenPHP started")
	fmt.Fprintf(os.Stderr, "Serving .%s domains on https (port 443) and http (port 80)\n", tld)

	// Create the server manager and reconcile secondary instances.
	sup := supervisor.New()
	manager = NewServerManager(mainFP, sup)
	defer func() {
		manager.Shutdown()
		manager = nil
	}()

	if err := manager.Reconcile(); err != nil {
		fmt.Fprintf(os.Stderr, "Warning: initial reconcile: %v\n", err)
	}

	// Start file watcher for pv.yml changes in linked projects.
	projectWatcher, watcherErr := watcher.New()
	if watcherErr != nil {
		fmt.Fprintf(os.Stderr, "Warning: cannot start file watcher: %v\n", watcherErr)
	} else {
		for _, project := range reg.List() {
			if err := projectWatcher.Watch(project.Name, project.Path); err != nil {
				fmt.Fprintf(os.Stderr, "Warning: cannot watch %s: %v\n", project.Name, err)
			}
		}
		activeWatcher = projectWatcher
		defer func() {
			activeWatcher = nil
			projectWatcher.Close()
		}()

		go handleWatcherEvents(projectWatcher)
	}

	// Boot Colima and recover service containers in the background.
	// This avoids blocking DNS + FrankenPHP startup on the ~15s VM boot.
	colimaCtx, colimaCancel := context.WithCancel(context.Background())
	defer colimaCancel()
	dockerCount := 0
	for _, inst := range reg.ListServices() {
		if inst.Kind != "binary" {
			dockerCount++
		}
	}
	if colima.IsInstalled() && dockerCount > 0 {
		go bootColimaAndRecover(colimaCtx, settings.Defaults.VM)
	}

	// Start background package updater.
	pkgCtx, pkgCancel := context.WithCancel(context.Background())
	defer pkgCancel()
	packages.StartBackgroundUpdater(pkgCtx, &http.Client{}, 24*time.Hour)

	// Wait for signals or child exit.
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
	defer signal.Stop(sigCh)

	return waitForEvent(sigCh, dnsErr, mainFP)
}

// waitForEvent blocks until a shutdown signal, DNS error, or main FrankenPHP exit.
// SIGHUP triggers a reconcile and continues the loop.
func waitForEvent(sigCh chan os.Signal, dnsErr chan error, mainFP *FrankenPHP) error {
	for {
		select {
		case sig := <-sigCh:
			if sig == syscall.SIGHUP {
				fmt.Fprintf(os.Stderr, "Received SIGHUP, reconciling...\n")
				if manager != nil {
					func() {
						defer func() {
							if r := recover(); r != nil {
								fmt.Fprintf(os.Stderr, "CRITICAL: reconcile panicked: %v\n", r)
							}
						}()
						if err := manager.Reconcile(); err != nil {
							fmt.Fprintf(os.Stderr, "Warning: reconcile failed: %v\n", err)
						}
					}()
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

// IsRunning checks if a pv supervisor process is currently running.
func IsRunning() bool {
	pid, err := ReadPID()
	if err != nil {
		return false
	}
	proc, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	// Signal 0 checks if process exists without sending a signal.
	return proc.Signal(syscall.Signal(0)) == nil
}

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
	if proc.Signal(syscall.Signal(0)) != nil {
		return nil // process doesn't exist
	}
	return proc.Signal(syscall.SIGHUP)
}

// ReadPID reads the PID from the PID file.
func ReadPID() (int, error) {
	data, err := os.ReadFile(config.PidFilePath())
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(strings.TrimSpace(string(data)))
}

func writePID() error {
	return os.WriteFile(config.PidFilePath(), []byte(strconv.Itoa(os.Getpid())), 0600)
}

func removePID() {
	if err := os.Remove(config.PidFilePath()); err != nil && !os.IsNotExist(err) {
		fmt.Fprintf(os.Stderr, "Warning: cannot remove PID file: %v\n", err)
	}
}

// handleWatcherEvents processes pv.yml change events, re-resolves PHP versions,
// updates the registry, and triggers a reconcile to start/stop secondaries.
func handleWatcherEvents(w *watcher.Watcher) {
	for event := range w.Events() {
		reg, err := registry.Load()
		if err != nil {
			fmt.Fprintf(os.Stderr, "Watcher: cannot load registry: %v\n", err)
			continue
		}
		project := reg.Find(event.ProjectName)
		if project == nil {
			continue
		}

		settings, err := config.LoadSettings()
		if err != nil {
			fmt.Fprintf(os.Stderr, "Watcher: cannot load settings: %v\n", err)
			continue
		}
		globalPHP := settings.Defaults.PHP

		var newPHP string
		switch event.Type {
		case watcher.ConfigChanged, watcher.ConfigDeleted:
			// Re-resolve PHP version (checks pv.yml -> composer.json -> global).
			if v, err := phpenv.ResolveVersion(event.ProjectPath); err == nil && v != "" {
				newPHP = v
			} else {
				if err != nil {
					fmt.Fprintf(os.Stderr, "Watcher: cannot resolve PHP version for %s: %v (falling back to global)\n", event.ProjectName, err)
				}
				newPHP = globalPHP
			}
		}

		if newPHP != "" && newPHP != project.PHP {
			// Ensure the version is installed before applying the change.
			wasInstalled := phpenv.IsInstalled(newPHP)
			if err := phpenv.EnsureInstalled(newPHP); err != nil {
				fmt.Fprintf(os.Stderr, "Watcher: cannot install PHP %s: %v (keeping %s)\n", newPHP, err, project.PHP)
				continue
			}
			if !wasInstalled {
				fmt.Fprintf(os.Stderr, "Watcher: PHP %s installed\n", newPHP)
			}

			fmt.Fprintf(os.Stderr, "Watcher: %s PHP version changed %s -> %s\n", event.ProjectName, project.PHP, newPHP)
			for i := range reg.Projects {
				if reg.Projects[i].Name == event.ProjectName {
					reg.Projects[i].PHP = newPHP
					break
				}
			}
			if err := reg.Save(); err != nil {
				fmt.Fprintf(os.Stderr, "Watcher: cannot save registry: %v\n", err)
				continue
			}
			if manager != nil {
				if err := manager.Reconcile(); err != nil {
					fmt.Fprintf(os.Stderr, "Watcher: reconcile failed: %v\n", err)
				}
			}
		}
	}
}

// WatchProject adds a project directory to the active file watcher.
// Safe to call when no watcher is running (e.g. server not started).
// Note: this is a no-op from CLI processes (link/unlink) since activeWatcher
// is only set in the daemon's Start(). Projects linked after server start
// will be watched on next server restart.
func WatchProject(name, path string) {
	if activeWatcher != nil {
		if err := activeWatcher.Watch(name, path); err != nil {
			fmt.Fprintf(os.Stderr, "Warning: cannot watch %s for config changes: %v\n", name, err)
		}
	}
}

// bootColimaAndRecover ensures the Colima VM is running and then recovers
// any stopped service containers. Runs in a background goroutine so it doesn't
// block DNS + FrankenPHP startup. Retries once on failure before giving up.
// The context is cancelled when the daemon shuts down.
func bootColimaAndRecover(ctx context.Context, vm config.VMConfig) {
	fmt.Fprintf(os.Stderr, "Starting Colima VM (registered services require Docker)...\n")

	// Ensure Colima VM is running. EnsureRunning already has internal recovery
	// (force-stop + delete + restart), so one retry here covers transient issues
	// like the socket not being ready immediately after boot.
	originalErr := colima.EnsureRunning(vm)
	if originalErr != nil {
		fmt.Fprintf(os.Stderr, "Warning: Colima start failed (%v), retrying in 10s...\n", originalErr)
		select {
		case <-time.After(10 * time.Second):
		case <-ctx.Done():
			return
		}
		if retryErr := colima.EnsureRunning(vm); retryErr != nil {
			fmt.Fprintf(os.Stderr, "Warning: Colima failed after retry.\n  Original: %v\n  Retry: %v\nServices unavailable. Run 'pv service:start' to try again.\n", originalErr, retryErr)
			return
		}
	}

	if ctx.Err() != nil {
		return
	}
	fmt.Fprintf(os.Stderr, "Colima VM running\n")

	// Reload registry to get current state — the snapshot from daemon start
	// may be stale since Colima boot takes 10-15 seconds.
	reg, err := registry.Load()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Warning: cannot reload registry for service recovery: %v\n", err)
		return
	}

	if len(reg.ListServices()) == 0 {
		return
	}

	// Recover service containers.
	engine, engineErr := container.NewEngine(config.ColimaSocketPath())
	if engineErr != nil {
		fmt.Fprintf(os.Stderr, "Warning: cannot connect to Docker for service recovery: %v\n", engineErr)
		return
	}
	defer engine.Close()

	for _, key := range colima.ServicesToRecover(reg) {
		if ctx.Err() != nil {
			return
		}
		svcName, version := services.ParseServiceKey(key)
		svc, lookupErr := services.Lookup(svcName)
		if lookupErr != nil {
			fmt.Fprintf(os.Stderr, "Warning: cannot recover service %s: %v\n", key, lookupErr)
			continue
		}
		name := svc.ContainerName(version)
		running, runErr := engine.IsRunning(ctx, name)
		if runErr != nil {
			fmt.Fprintf(os.Stderr, "Warning: cannot check status for %s: %v\n", name, runErr)
			continue
		}
		if !running {
			if err := engine.Start(ctx, name); err != nil {
				fmt.Fprintf(os.Stderr, "Warning: could not recover container %s: %v\n", name, err)
			} else {
				fmt.Fprintf(os.Stderr, "Recovered service container %s\n", name)
			}
		}
	}
}

// UnwatchProject removes a project directory from the active file watcher.
// Safe to call when no watcher is running.
func UnwatchProject(path string) {
	if activeWatcher != nil {
		if err := activeWatcher.Unwatch(path); err != nil {
			fmt.Fprintf(os.Stderr, "Warning: cannot unwatch project: %v\n", err)
		}
	}
}
