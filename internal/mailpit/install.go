package mailpit

import (
	"fmt"
	"net/http"
	"os"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func Install(client *http.Client, version string) error {
	return InstallProgress(client, version, nil)
}

func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	latest, err := binaries.FetchLatestVersion(client, Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
	}
	if err := binaries.InstallBinaryProgress(client, Binary(), latest, progress); err != nil {
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
	if err := os.MkdirAll(config.ServiceDataDir(ServiceKey(), version), 0o755); err != nil {
		return fmt.Errorf("create mailpit data dir: %w", err)
	}
	return SetWanted(version, WantedRunning)
}
