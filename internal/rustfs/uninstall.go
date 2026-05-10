package rustfs

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// Uninstall stops, unregisters, and removes the rustfs binary. When
// deleteData is true the data directory is also wiped (postgres-style
// :uninstall). Linked Laravel projects are unbound and their .env files
// get fallback values applied. The caller is responsible for confirming
// with the user before invoking when deleteData == true.
//
// The registry entry is removed last so that a failure in any earlier
// step (file removal, data deletion) leaves a state where re-running
// :uninstall is meaningful — the entry still exists, the operation can
// be retried.
func Uninstall(deleteData bool) error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	inst, ok := reg.Services[ServiceKey()]
	if !ok {
		return fmt.Errorf("%s not registered (run `pv %s:install` first)", ServiceKey(), Binary().Name)
	}

	// Stop the supervised process first. If the daemon is up, set
	// Enabled=false, signal, and wait for the process to exit before
	// touching any files. If the daemon isn't running, this branch is
	// inert (the supervisor isn't holding any handles).
	disabled := false
	inst.Enabled = &disabled
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			return fmt.Errorf("could not signal daemon to stop %s: %w", ServiceKey(), err)
		}
		if err := WaitStopped(30 * time.Second); err != nil {
			ui.Subtle(fmt.Sprintf("Could not confirm %s stopped: %v (continuing)", DisplayName(), err))
		}
	}

	binPath := filepath.Join(config.InternalBinDir(), Binary().Name)
	if err := os.Remove(binPath); err != nil && !os.IsNotExist(err) {
		ui.Subtle(fmt.Sprintf("Could not remove %s: %v (file left behind)", binPath, err))
	}
	if vs, vsErr := binaries.LoadVersions(); vsErr != nil {
		ui.Subtle(fmt.Sprintf("Could not load versions file: %v (manifest may be stale)", vsErr))
	} else {
		vs.Set(Binary().Name, "")
		if err := vs.Save(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not save versions file: %v", err))
		}
	}
	if deleteData {
		dataDir := config.ServiceDataDir(ServiceKey(), "latest")
		if err := os.RemoveAll(dataDir); err != nil {
			return fmt.Errorf("cannot delete data: %w", err)
		}
	}

	// Apply fallbacks while project bindings are still present, then
	// unbind. Fallbacks look up linked projects via the per-project
	// flags, so the order matters.
	ApplyFallbacksToLinkedProjects(reg)
	reg.UnbindService(ServiceKey())

	if err := reg.RemoveService(ServiceKey()); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
	}
	return nil
}
