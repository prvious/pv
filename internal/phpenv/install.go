package phpenv

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"runtime"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

const (
	// releaseRepo is the GitHub repository hosting custom FrankenPHP + PHP CLI builds.
	releaseRepo = "prvious/pv"
)

// Install downloads and installs a PHP version (FrankenPHP + PHP CLI).
// The phpVersion is a major.minor string like "8.4".
func Install(client *http.Client, phpVersion string) error {
	return InstallProgress(client, phpVersion, nil)
}

// InstallProgress downloads and installs a PHP version with optional progress reporting.
func InstallProgress(client *http.Client, phpVersion string, progress binaries.ProgressFunc) error {
	versionDir := config.PhpVersionDir(phpVersion)
	if err := os.MkdirAll(versionDir, 0755); err != nil {
		return fmt.Errorf("cannot create version directory: %w", err)
	}

	// 1. Find the latest FrankenPHP release tag from prvious/pv.
	tag, err := latestReleaseTag(client)
	if err != nil {
		return fmt.Errorf("cannot find latest release: %w", err)
	}

	// 2. Download FrankenPHP binary for this PHP version.
	assetName, err := frankenphpAssetName(phpVersion)
	if err != nil {
		return err
	}

	fpURL := fmt.Sprintf("https://github.com/%s/releases/download/%s/%s", releaseRepo, tag, assetName)
	fpDest := FrankenPHPPath(phpVersion)

	if err := binaries.DownloadProgress(client, fpURL, fpDest, progress); err != nil {
		return fmt.Errorf("download FrankenPHP: %w", err)
	}
	if err := binaries.MakeExecutable(fpDest); err != nil {
		return err
	}

	// 3. Download PHP CLI from the same release. Built alongside FrankenPHP
	// so both binaries share an identical extension set.
	phpURL, err := phpCLIURL(tag, phpVersion)
	if err != nil {
		return err
	}

	phpArchive := fpDest + ".php.tar.gz"
	phpDest := PHPPath(phpVersion)

	if err := binaries.DownloadProgress(client, phpURL, phpArchive, progress); err != nil {
		return fmt.Errorf("download PHP CLI: %w", err)
	}

	if err := binaries.ExtractTarGz(phpArchive, phpDest, "php"); err != nil {
		return fmt.Errorf("extract PHP CLI: %w", err)
	}
	os.Remove(phpArchive)

	if err := binaries.MakeExecutable(phpDest); err != nil {
		return err
	}

	return nil
}

// latestReleaseTag fetches the latest release tag from the prvious/pv repo.
func latestReleaseTag(client *http.Client) (string, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/releases/latest", releaseRepo)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	binaries.SetGitHubHeaders(req)

	resp, err := client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("GitHub API returned HTTP %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}

	var release struct {
		TagName string `json:"tag_name"`
	}
	if err := json.Unmarshal(body, &release); err != nil {
		return "", err
	}
	return release.TagName, nil
}

// frankenphpAssetName returns the release asset name for the current platform.
// Format: frankenphp-{platform}-php{version}
func frankenphpAssetName(phpVersion string) (string, error) {
	platform, err := platformName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("frankenphp-%s-php%s", platform, phpVersion), nil
}

var platformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "mac-arm64",
		"amd64": "mac-x86_64",
	},
	"linux": {
		"amd64": "linux-x86_64",
		"arm64": "linux-aarch64",
	},
}

func platformName() (string, error) {
	archMap, ok := platformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS: %s", runtime.GOOS)
	}
	name, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return name, nil
}

// phpCLIURL returns the release asset URL for the static PHP CLI tarball.
// Format: php-{platform}-php{version}.tar.gz (containing a `php` binary).
func phpCLIURL(tag, phpVersion string) (string, error) {
	platform, err := platformName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/%s/releases/download/%s/php-%s-php%s.tar.gz", releaseRepo, tag, platform, phpVersion), nil
}
