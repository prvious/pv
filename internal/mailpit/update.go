package mailpit

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
)

func Update(client *http.Client, version string) error {
	return UpdateProgress(client, version, nil)
}

func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if !IsInstalled(version) {
		return fmt.Errorf("mailpit %s is not installed", version)
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
	return vs.Save()
}
