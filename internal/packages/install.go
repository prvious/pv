package packages

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

type gitHubRelease struct {
	TagName string        `json:"tag_name"`
	Assets  []gitHubAsset `json:"assets"`
}

type gitHubAsset struct {
	Name        string `json:"name"`
	DownloadURL string `json:"browser_download_url"`
}

// fetchLatestRelease queries GitHub for the latest release tag and asset download URL.
func fetchLatestRelease(client *http.Client, pkg Package) (tag, downloadURL string, err error) {
	req, err := http.NewRequest("GET", pkg.LatestReleaseURL(), nil)
	if err != nil {
		return "", "", err
	}
	binaries.SetGitHubHeaders(req)

	resp, err := client.Do(req)
	if err != nil {
		return "", "", fmt.Errorf("fetch latest release for %s: %w", pkg.Name, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", "", fmt.Errorf("GitHub API returned HTTP %d for %s", resp.StatusCode, pkg.Repo)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", "", err
	}

	var release gitHubRelease
	if err := json.Unmarshal(body, &release); err != nil {
		return "", "", fmt.Errorf("parse GitHub response for %s: %w", pkg.Name, err)
	}

	for _, asset := range release.Assets {
		if asset.Name == pkg.Asset {
			return release.TagName, asset.DownloadURL, nil
		}
	}

	return "", "", fmt.Errorf("asset %q not found in %s release %s", pkg.Asset, pkg.Repo, release.TagName)
}

// Install downloads a package PHAR, symlinks it, and records the version.
// Returns the installed version tag.
func Install(client *http.Client, pkg Package, progress binaries.ProgressFunc) (string, error) {
	if err := config.EnsureDirs(); err != nil {
		return "", err
	}

	tag, downloadURL, err := fetchLatestRelease(client, pkg)
	if err != nil {
		return "", err
	}

	if err := binaries.DownloadProgress(client, downloadURL, pkg.PharPath(), progress); err != nil {
		return "", fmt.Errorf("download %s: %w", pkg.Name, err)
	}

	if err := binaries.MakeExecutable(pkg.PharPath()); err != nil {
		return "", err
	}

	// Create symlink (remove existing first to handle reinstalls).
	os.Remove(pkg.SymlinkPath())
	if err := os.Symlink(pkg.PharPath(), pkg.SymlinkPath()); err != nil {
		return "", fmt.Errorf("symlink %s: %w", pkg.Name, err)
	}

	// Record installed version.
	vs, err := binaries.LoadVersions()
	if err != nil {
		return "", err
	}
	vs.Set(pkg.Name, tag)
	if err := vs.Save(); err != nil {
		return "", err
	}

	return tag, nil
}

// InstallAll installs all managed packages.
func InstallAll(client *http.Client, progress binaries.ProgressFunc) error {
	for _, pkg := range Managed {
		if _, err := Install(client, pkg, progress); err != nil {
			return err
		}
	}
	return nil
}
