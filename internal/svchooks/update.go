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
// service must already be installed (registered with Kind="binary").
// On success the daemon is signaled so the supervisor restarts the
// process with the new binary; if the signal fails, the function
// returns a non-nil error so the exit code reflects that the supervisor
// is still running the previous version.
func Update(reg *registry.Registry, svc services.BinaryService) error {
	if _, err := requireBinaryEntry(reg, svc); err != nil {
		return err
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
			return fmt.Errorf(
				"%s binary updated to %s, but the daemon is still running the previous version (run `pv restart`): %w",
				svc.DisplayName(), latest, err,
			)
		}
	}
	ui.Success(fmt.Sprintf("%s updated to %s", svc.DisplayName(), latest))
	return nil
}
