package packages

import (
	"context"
	"fmt"
	"net/http"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update updates a managed package and reports whether the version changed.
// For PHAR packages, checks the latest GitHub release first and only downloads if newer.
// For Composer packages, runs composer global update and detects version changes after.
func Update(ctx context.Context, client *http.Client, pkg Package) (updated bool, version string, err error) {
	if err := config.EnsureDirs(); err != nil {
		return false, "", err
	}

	switch pkg.Method {
	case MethodComposer:
		return updateViaComposer(ctx, pkg)
	case MethodPHAR:
		return updateViaPHAR(ctx, client, pkg)
	default:
		return false, "", fmt.Errorf("unknown install method %d for package %s", pkg.Method, pkg.Name)
	}
}

func updateViaComposer(ctx context.Context, pkg Package) (bool, string, error) {
	vs, err := binaries.LoadVersions()
	if err != nil {
		return false, "", err
	}
	installed := vs.Get(pkg.Name)

	version, err := composerGlobalUpdate(ctx, pkg)
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

func updateViaPHAR(ctx context.Context, client *http.Client, pkg Package) (bool, string, error) {
	tag, downloadURL, err := fetchLatestRelease(ctx, client, pkg)
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
		return false, "", fmt.Errorf("save version after updating %s to %s (binary already on disk): %w", pkg.Name, tag, err)
	}

	return true, tag, nil
}

// UpdateAll checks and updates all managed packages.
// Returns the names of packages that were successfully updated.
// On error, returns the packages updated so far along with the error.
func UpdateAll(ctx context.Context, client *http.Client) ([]string, error) {
	var updated []string
	for _, pkg := range Managed {
		wasUpdated, _, err := Update(ctx, client, pkg)
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
