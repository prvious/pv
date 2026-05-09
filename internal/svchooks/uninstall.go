package svchooks

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
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// Uninstall stops, unregisters, and removes svc's binary. When deleteData
// is true the data directory is also wiped (postgres-style :uninstall).
// Linked Laravel projects are unbound and their .env files get fallback
// values applied. The caller is responsible for confirming with the user
// before invoking when deleteData == true.
//
// The registry entry is removed last so that a failure in any earlier
// step (file removal, data deletion) leaves a state where re-running
// :uninstall is meaningful — the entry still exists, the operation can
// be retried.
func Uninstall(svc services.BinaryService, reg *registry.Registry, deleteData bool) error {
	name := svc.Name()
	inst, err := requireBinaryEntry(reg, svc)
	if err != nil {
		return err
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
			return fmt.Errorf("could not signal daemon to stop %s: %w", name, err)
		}
		if err := WaitStopped(svc, 30*time.Second); err != nil {
			ui.Subtle(fmt.Sprintf("Could not confirm %s stopped: %v (continuing)", svc.DisplayName(), err))
		}
	}

	// On-disk cleanup. Binary removal and versions-manifest pruning are
	// best-effort with a Subtle warning so a partial failure can be
	// retried. Data deletion is the user's explicit ask, so a failure
	// there aborts the operation.
	binPath := filepath.Join(config.InternalBinDir(), svc.Binary().Name)
	if err := os.Remove(binPath); err != nil && !os.IsNotExist(err) {
		ui.Subtle(fmt.Sprintf("Could not remove %s: %v (file left behind)", binPath, err))
	}
	if vs, vsErr := binaries.LoadVersions(); vsErr != nil {
		ui.Subtle(fmt.Sprintf("Could not load versions file: %v (manifest may be stale)", vsErr))
	} else {
		vs.Set(svc.Binary().Name, "")
		if err := vs.Save(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not save versions file: %v", err))
		}
	}
	if deleteData {
		dataDir := config.ServiceDataDir(name, "latest")
		if err := os.RemoveAll(dataDir); err != nil {
			return fmt.Errorf("cannot delete data: %w", err)
		}
	}

	// Apply fallbacks while project bindings are still present, then
	// unbind. Fallbacks look up linked projects via the per-project
	// flags, so the order matters.
	ApplyFallbacksToLinkedProjects(reg, name)
	reg.UnbindService(name)

	if err := reg.RemoveService(name); err != nil {
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
