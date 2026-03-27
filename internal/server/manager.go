package server

import (
	"fmt"
	"os"
	"strings"
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
	var startErrors []string
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
				startErrors = append(startErrors, fmt.Sprintf("PHP %s: %v", version, err))
				continue
			}
			m.secondaries[version] = newFP
		}
	}

	// Reload the main FrankenPHP to pick up new Caddyfile.
	if err := Reload(); err != nil {
		return fmt.Errorf("reconcile: reload main FrankenPHP: %w", err)
	}

	if len(startErrors) > 0 {
		return fmt.Errorf("reconcile: %d secondary instance(s) failed: %s", len(startErrors), strings.Join(startErrors, "; "))
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
func (m *ServerManager) RunningVersions() []string {
	m.mu.Lock()
	defer m.mu.Unlock()

	var versions []string
	for v := range m.secondaries {
		versions = append(versions, v)
	}
	return versions
}
