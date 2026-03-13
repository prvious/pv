package packages

import (
	"net/http"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update checks if a package has a newer version and downloads it if so.
// Returns whether an update occurred and the current/new version tag.
// The symlink is not touched — the PHAR is replaced in-place.
func Update(client *http.Client, pkg Package) (updated bool, version string, err error) {
	if err := config.EnsureDirs(); err != nil {
		return false, "", err
	}

	tag, downloadURL, err := fetchLatestRelease(client, pkg)
	if err != nil {
		return false, "", err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return false, "", err
	}

	installed := vs.Get(pkg.Name)
	if normalizeVersion(installed) == normalizeVersion(tag) {
		return false, tag, nil
	}

	if err := binaries.DownloadProgress(client, downloadURL, pkg.PharPath(), nil); err != nil {
		return false, "", err
	}

	if err := binaries.MakeExecutable(pkg.PharPath()); err != nil {
		return false, "", err
	}

	vs.Set(pkg.Name, tag)
	if err := vs.Save(); err != nil {
		return false, "", err
	}

	return true, tag, nil
}

// UpdateAll checks and updates all managed packages.
// Returns a slice of package names that were updated.
func UpdateAll(client *http.Client) ([]string, error) {
	var updated []string
	for _, pkg := range Managed {
		wasUpdated, _, err := Update(client, pkg)
		if err != nil {
			return updated, err
		}
		if wasUpdated {
			updated = append(updated, pkg.Name)
		}
	}
	return updated, nil
}

func normalizeVersion(v string) string {
	return strings.TrimPrefix(v, "v")
}
