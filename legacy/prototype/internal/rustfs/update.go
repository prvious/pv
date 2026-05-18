package rustfs

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
		return fmt.Errorf("rustfs %s is not installed", version)
	}
	if err := installArchive(client, version, progress); err != nil {
		return err
	}
	return recordInstalledVersion(version)
}
