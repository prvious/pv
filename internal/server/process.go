package server

import (
	"fmt"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/watcher"
)

// activeWatcher holds the file watcher for pv.yml changes in linked projects.
var activeWatcher *watcher.Watcher

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

	// Regenerate all caddy configs before starting.
	settings, err := config.LoadSettings()
	if err != nil {
		return fmt.Errorf("cannot load settings: %w", err)
	}
	globalPHP := settings.Defaults.PHP

	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}

	if err := caddy.GenerateAllConfigs(reg.List(), globalPHP); err != nil {
		return fmt.Errorf("cannot generate caddy configs: %w", err)
	}

	// Start DNS server in a goroutine.
	dnsServer := NewDNSServer(tld)
	dnsErr := make(chan error, 1)
	go func() { dnsErr <- dnsServer.Start() }()
	defer dnsServer.Shutdown()

	fmt.Printf("DNS server listening on %s\n", dnsServer.Addr)

	// Start main FrankenPHP.
	mainFP, err := StartFrankenPHP()
	if err != nil {
		return fmt.Errorf("cannot start FrankenPHP: %w", err)
	}
	defer mainFP.Stop()

	fmt.Println("FrankenPHP started")
	fmt.Printf("Serving .%s domains on https (port 443) and http (port 80)\n", tld)

	// Start secondary FrankenPHP instances for non-global PHP versions.
	activeVersions := caddy.ActiveVersions(reg.List(), globalPHP)
	var secondaries []*FrankenPHP
	for version := range activeVersions {
		port := config.PortForVersion(version)
		fmt.Printf("Starting FrankenPHP for PHP %s on port %d...\n", version, port)
		fp, err := StartVersionFrankenPHP(version)
		if err != nil {
			fmt.Printf("Warning: cannot start FrankenPHP for PHP %s: %v\n", version, err)
			continue
		}
		secondaries = append(secondaries, fp)
		fmt.Printf("FrankenPHP (PHP %s) started on port %d\n", version, port)
	}
	defer func() {
		for _, fp := range secondaries {
			fp.Stop()
		}
	}()

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

		go handleWatcherEvents(projectWatcher, globalPHP)
	}

	// Wait for signals or child exit.
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)

	return waitForEvent(sigCh, dnsErr, mainFP, secondaries)
}

// waitForEvent blocks until a signal, DNS error, or any FrankenPHP process exits.
func waitForEvent(sigCh chan os.Signal, dnsErr chan error, mainFP *FrankenPHP, secondaries []*FrankenPHP) error {
	// Since Go doesn't support dynamic select, we merge secondary done channels
	// into a single channel.
	merged := make(chan string, 1) // version string or "" for non-secondary event
	done := make(chan struct{})
	defer close(done)

	// Watch secondaries.
	for _, fp := range secondaries {
		go func(f *FrankenPHP) {
			select {
			case err := <-f.Done():
				if err != nil {
					fmt.Printf("FrankenPHP (PHP %s) exited: %v\n", f.Version(), err)
				}
				select {
				case merged <- f.Version():
				case <-done:
				}
			case <-done:
			}
		}(fp)
	}

	select {
	case sig := <-sigCh:
		fmt.Printf("\nReceived %s, shutting down...\n", sig)
	case err := <-dnsErr:
		if err != nil {
			fmt.Printf("DNS server error: %v\n", err)
		}
	case err := <-mainFP.Done():
		if err != nil {
			fmt.Printf("FrankenPHP exited: %v\n", err)
		}
	case v := <-merged:
		fmt.Printf("Secondary FrankenPHP (PHP %s) exited\n", v)
	}
	return nil
}

// ReconfigureServer regenerates all caddy configs and restarts/reloads as needed.
// Called after pv use, pv link, pv unlink when the server is running.
func ReconfigureServer() error {
	settings, err := config.LoadSettings()
	if err != nil {
		return err
	}

	reg, err := registry.Load()
	if err != nil {
		return err
	}

	// Regenerate all site configs and Caddyfiles.
	if err := caddy.GenerateAllConfigs(reg.List(), settings.Defaults.PHP); err != nil {
		return err
	}

	// Reload the main FrankenPHP.
	return Reload()
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

// ReadPID reads the PID from the PID file.
func ReadPID() (int, error) {
	data, err := os.ReadFile(config.PidFilePath())
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(strings.TrimSpace(string(data)))
}

func writePID() error {
	return os.WriteFile(config.PidFilePath(), []byte(strconv.Itoa(os.Getpid())), 0644)
}

func removePID() {
	os.Remove(config.PidFilePath())
}

// handleWatcherEvents processes pv.yml change events, re-resolves PHP versions,
// updates the registry, and reconfigures the server when needed.
func handleWatcherEvents(w *watcher.Watcher, globalPHP string) {
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

		var newPHP string
		switch event.Type {
		case watcher.ConfigChanged, watcher.ConfigDeleted:
			// Re-resolve PHP version (checks pv.yml -> composer.json -> global).
			if v, err := phpenv.ResolveVersion(event.ProjectPath); err == nil && v != "" {
				newPHP = v
			} else {
				newPHP = globalPHP
			}
		}

		if newPHP != "" && newPHP != project.PHP {
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
			if err := ReconfigureServer(); err != nil {
				fmt.Fprintf(os.Stderr, "Watcher: cannot reconfigure server: %v\n", err)
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

// UnwatchProject removes a project directory from the active file watcher.
// Safe to call when no watcher is running.
func UnwatchProject(path string) {
	if activeWatcher != nil {
		if err := activeWatcher.Unwatch(path); err != nil {
			fmt.Fprintf(os.Stderr, "Warning: cannot unwatch project: %v\n", err)
		}
	}
}
