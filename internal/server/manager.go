package server

import (
	"context"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	mailpitproc "github.com/prvious/pv/internal/mailpit/proc"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	rustfsproc "github.com/prvious/pv/internal/rustfs/proc"
	"github.com/prvious/pv/internal/supervisor"
)

// ServerManager owns the main and secondary FrankenPHP instances.
// Reconcile() is the single entry point for syncing running instances
// against the current config state on disk.
type ServerManager struct {
	mu          sync.Mutex
	main        *FrankenPHP
	secondaries map[string]*FrankenPHP // version -> instance
	supervisor  *supervisor.Supervisor // binary services; may be nil in tests
}

// NewServerManager creates a manager with the given main FrankenPHP instance
// and an optional supervisor for native binary services.
func NewServerManager(main *FrankenPHP, sup *supervisor.Supervisor) *ServerManager {
	return &ServerManager{
		main:        main,
		secondaries: make(map[string]*FrankenPHP),
		supervisor:  sup,
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

	// Phase 2: binary services. Errors are logged and also folded into the
	// return value so callers (service:add, etc.) can see that a binary
	// failed to come up rather than getting a false "reconciled" signal.
	binaryErr := m.reconcileBinaryServices(context.Background())
	if binaryErr != nil {
		fmt.Fprintf(os.Stderr, "Reconcile: %v\n", binaryErr)
	}

	// Phase 3: refresh daemon-status snapshot.
	if err := writeDaemonStatus(m.supervisor); err != nil {
		fmt.Fprintf(os.Stderr, "Reconcile: write daemon-status: %v\n", err)
	}

	// Combine secondary-instance and binary-service errors for the caller.
	var parts []string
	if len(startErrors) > 0 {
		parts = append(parts, fmt.Sprintf("secondary instances: %s", strings.Join(startErrors, "; ")))
	}
	if binaryErr != nil {
		parts = append(parts, fmt.Sprintf("binary services: %v", binaryErr))
	}
	if len(parts) > 0 {
		return fmt.Errorf("reconcile failed: %s", strings.Join(parts, "; "))
	}
	return nil
}

// Shutdown stops all secondary FrankenPHP instances.
// The main instance is stopped separately via its own defer in Start().
func (m *ServerManager) Shutdown() {
	if m.supervisor != nil {
		m.supervisor.StopAll(10 * time.Second)
	}

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

// reconcileBinaryServices brings supervisor state in line with the wanted
// set computed from four sources:
//  1. registry: single-version services (rustfs via "s3", mailpit via "mail"),
//     gated on the registry Enabled flag.
//  2. internal/postgres: multi-version, on-disk + state.json driven.
//  3. internal/mysql:    multi-version, on-disk + state.json driven.
//  4. internal/redis:    single-version, on-disk + state.json driven.
//
// The diff/start/stop loop is shared across all four sources.
func (m *ServerManager) reconcileBinaryServices(ctx context.Context) error {
	if m.supervisor == nil {
		return nil
	}

	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("reconcile binary: load registry: %w", err)
	}

	// wanted: supervisorKey -> buildable supervisor.Process.
	wanted := map[string]supervisor.Process{}
	var startErrors []string

	// Source 1a — rustfs.
	if entry := reg.Services["s3"]; entry != nil {
		if entry.Enabled == nil || *entry.Enabled {
			proc, err := rustfsproc.BuildSupervisorProcess()
			if err != nil {
				startErrors = append(startErrors, fmt.Sprintf("s3: build: %v", err))
			} else {
				wanted[rustfsproc.Binary().Name] = proc
			}
		}
	}

	// Source 1b — mailpit.
	if entry := reg.Services["mail"]; entry != nil {
		if entry.Enabled == nil || *entry.Enabled {
			proc, err := mailpitproc.BuildSupervisorProcess()
			if err != nil {
				startErrors = append(startErrors, fmt.Sprintf("mail: build: %v", err))
			} else {
				wanted[mailpitproc.Binary().Name] = proc
			}
		}
	}

	// Source 2 — postgres, multi-version.
	pgMajors, pgErr := postgres.WantedMajors()
	if pgErr != nil {
		fmt.Fprintf(os.Stderr, "reconcile binary: postgres.WantedMajors: %v\n", pgErr)
	}
	for _, major := range pgMajors {
		proc, err := postgres.BuildSupervisorProcess(major)
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("postgres-%s: build: %v", major, err))
			continue
		}
		wanted["postgres-"+major] = proc
	}

	// Source 3 — mysql, multi-version.
	myVersions, myErr := mysql.WantedVersions()
	if myErr != nil {
		fmt.Fprintf(os.Stderr, "reconcile binary: mysql.WantedVersions: %v\n", myErr)
	}
	for _, version := range myVersions {
		proc, err := mysql.BuildSupervisorProcess(version)
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("mysql-%s: build: %v", version, err))
			continue
		}
		wanted["mysql-"+version] = proc
	}

	// Source 4 — redis, per-version, filesystem + state.json.
	redisVersions, err := redis.WantedVersions()
	if err != nil {
		startErrors = append(startErrors, fmt.Sprintf("redis: wanted: %v", err))
	}
	for _, version := range redisVersions {
		proc, err := redis.BuildSupervisorProcess(version)
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("redis-%s: build: %v", version, err))
		} else {
			wanted["redis-"+version] = proc
		}
	}

	// Diff: stop unneeded. If the postgres source failed, skip postgres-
	// prefixed keys — a transient state.json read error shouldn't kill
	// running postgres processes (the wanted set is incomplete, not empty).
	// Same transient-error guard for mysql.
	for _, supKey := range m.supervisor.SupervisedNames() {
		if _, ok := wanted[supKey]; ok {
			continue
		}
		if pgErr != nil && strings.HasPrefix(supKey, "postgres-") {
			continue
		}
		if myErr != nil && strings.HasPrefix(supKey, "mysql-") {
			continue
		}
		if err := m.supervisor.Stop(supKey, 10*time.Second); err != nil {
			fmt.Fprintf(os.Stderr, "reconcile binary: stop %s: %v\n", supKey, err)
		}
	}

	// Diff: start needed.
	for supKey, proc := range wanted {
		if m.supervisor.IsRunning(supKey) {
			continue
		}
		if err := m.supervisor.Start(ctx, proc); err != nil {
			startErrors = append(startErrors, fmt.Sprintf("%s: start: %v", supKey, err))
			continue
		}
	}

	if len(startErrors) > 0 {
		return fmt.Errorf("binary reconcile: %d service(s) failed: %s", len(startErrors), strings.Join(startErrors, "; "))
	}
	return nil
}
