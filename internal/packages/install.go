package packages

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"

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
func fetchLatestRelease(ctx context.Context, client *http.Client, pkg Package) (tag, downloadURL string, err error) {
	req, err := http.NewRequestWithContext(ctx, "GET", pkg.LatestReleaseURL(), nil)
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
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return "", "", fmt.Errorf("GitHub API returned HTTP %d for %s: %s", resp.StatusCode, pkg.Repo, strings.TrimSpace(string(body)))
	}

	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20)) // 1 MB limit
	if err != nil {
		return "", "", err
	}

	var release gitHubRelease
	if err := json.Unmarshal(body, &release); err != nil {
		return "", "", fmt.Errorf("parse GitHub response for %s: %w", pkg.Name, err)
	}

	if release.TagName == "" {
		return "", "", fmt.Errorf("empty tag in %s latest release", pkg.Repo)
	}

	for _, asset := range release.Assets {
		if asset.Name == pkg.Asset {
			return release.TagName, asset.DownloadURL, nil
		}
	}

	return "", "", fmt.Errorf("asset %q not found in %s release %s", pkg.Asset, pkg.Repo, release.TagName)
}

// Install installs a managed package, records the version, and returns the installed version string.
// For PHAR packages, downloads from GitHub releases and creates a symlink.
// For Composer packages, runs composer global require.
func Install(ctx context.Context, client *http.Client, pkg Package, progress binaries.ProgressFunc) (string, error) {
	if err := config.EnsureDirs(); err != nil {
		return "", err
	}

	switch pkg.Method {
	case MethodComposer:
		return installViaComposer(ctx, pkg)
	case MethodPHAR:
		return installViaPHAR(ctx, client, pkg, progress)
	default:
		return "", fmt.Errorf("unknown install method %d for package %s", pkg.Method, pkg.Name)
	}
}

func installViaComposer(ctx context.Context, pkg Package) (string, error) {
	version, err := composerGlobalRequire(ctx, pkg)
	if err != nil {
		return "", err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return "", err
	}
	vs.Set(pkg.Name, version)
	if err := vs.Save(); err != nil {
		return "", err
	}

	return version, nil
}

func installViaPHAR(ctx context.Context, client *http.Client, pkg Package, progress binaries.ProgressFunc) (string, error) {
	tag, downloadURL, err := fetchLatestRelease(ctx, client, pkg)
	if err != nil {
		return "", err
	}

	if err := binaries.DownloadProgress(client, downloadURL, pkg.PharPath(), progress); err != nil {
		return "", fmt.Errorf("download %s: %w", pkg.Name, err)
	}

	if err := binaries.MakeExecutable(pkg.PharPath()); err != nil {
		return "", err
	}

	// Remove existing symlink to handle reinstalls; ignore only "not exist".
	if err := os.Remove(pkg.SymlinkPath()); err != nil && !os.IsNotExist(err) {
		return "", fmt.Errorf("remove existing symlink for %s: %w", pkg.Name, err)
	}
	if err := os.Symlink(pkg.PharPath(), pkg.SymlinkPath()); err != nil {
		return "", fmt.Errorf("symlink %s: %w", pkg.Name, err)
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return "", err
	}
	vs.Set(pkg.Name, tag)
	if err := vs.Save(); err != nil {
		return "", fmt.Errorf("save version after installing %s %s (binary already on disk): %w", pkg.Name, tag, err)
	}

	return tag, nil
}

// InstallAll installs all managed packages.
func InstallAll(ctx context.Context, client *http.Client, progress binaries.ProgressFunc) error {
	for _, pkg := range Managed {
		if _, err := Install(ctx, client, pkg, progress); err != nil {
			return err
		}
	}
	return nil
}
