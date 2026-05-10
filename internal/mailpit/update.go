package mailpit

import (
	"fmt"
	"net/http"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

// Update re-downloads the mailpit binary to the latest upstream version.
// The service must already be installed. On success the daemon is
// signaled so the supervisor restarts the process with the new binary;
// if the signal fails, the function returns a non-nil error so the
// exit code reflects that the supervisor is still running the previous
// version.
func Update() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	if _, ok := reg.Services[serviceKey]; !ok {
		return fmt.Errorf("%s not registered (run `pv %s:install` first)", serviceKey, Binary().Name)
	}

	client := &http.Client{Timeout: 60 * time.Second}

	latest, err := binaries.FetchLatestVersion(client, Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
	}

	if err := ui.Step(fmt.Sprintf("Updating %s to %s...", Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			return fmt.Errorf(
				"%s binary updated to %s, but the daemon is still running the previous version (run `pv restart`): %w",
				DisplayName(), latest, err,
			)
		}
	}
	ui.Success(fmt.Sprintf("%s updated to %s", DisplayName(), latest))
	return nil
}
