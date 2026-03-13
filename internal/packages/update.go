package packages

import (
	"net/http"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update checks if a package has a newer version and updates it.
// For PHAR packages, the PHAR is replaced in-place (symlink untouched).
// For Composer packages, runs composer global update.
func Update(client *http.Client, pkg Package) (updated bool, version string, err error) {
	if err := config.EnsureDirs(); err != nil {
		return false, "", err
	}

	switch pkg.Method {
	case MethodComposer:
		return updateViaComposer(pkg)
	default:
		return updateViaPHAR(client, pkg)
	}
}

func updateViaComposer(pkg Package) (bool, string, error) {
	vs, err := binaries.LoadVersions()
	if err != nil {
		return false, "", err
	}
	installed := vs.Get(pkg.Name)

	version, err := composerGlobalUpdate(pkg)
	if err != nil {
		return false, "", err
	}

	if normalizeVersion(installed) == normalizeVersion(version) {
		return false, version, nil
	}

	vs.Set(pkg.Name, version)
	if err := vs.Save(); err != nil {
		return false, "", err
	}

	return true, version, nil
}

func updateViaPHAR(client *http.Client, pkg Package) (bool, string, error) {
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
