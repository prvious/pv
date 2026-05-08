package svchooks

import (
	"fmt"
	"net/http"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// Update re-downloads svc's binary to the latest upstream version. The
// service must already be installed (registered). On success the daemon
// is signaled so the supervisor restarts the process with the new binary.
func Update(reg *registry.Registry, svc services.BinaryService) error {
	name := svc.Name()
	if _, ok := reg.Services[name]; !ok {
		return fmt.Errorf("%s is not installed (run `pv %s:install`)", name, svc.Binary().Name)
	}

	client := &http.Client{Timeout: 60 * time.Second}

	latest, err := binaries.FetchLatestVersion(client, svc.Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", svc.Binary().DisplayName, err)
	}

	if err := ui.Step(fmt.Sprintf("Updating %s to %s...", svc.Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, svc.Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", svc.Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(svc.Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
	}
	ui.Success(fmt.Sprintf("%s updated to %s", svc.DisplayName(), latest))
	return nil
}
