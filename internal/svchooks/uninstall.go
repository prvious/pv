package svchooks

import (
	"fmt"
	"os"
	"path/filepath"

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
func Uninstall(svc services.BinaryService, reg *registry.Registry, deleteData bool) error {
	name := svc.Name()
	if _, ok := reg.Services[name]; !ok {
		return fmt.Errorf("%s not registered", name)
	}

	if err := reg.RemoveService(name); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

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

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
	}

	ApplyFallbacksToLinkedProjects(reg, name)
	reg.UnbindService(name)
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}
	return nil
}
