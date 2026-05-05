package phpenv

import (
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"runtime"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

const (
	// releaseRepo is the GitHub repository hosting custom FrankenPHP + PHP CLI builds.
	releaseRepo = "prvious/pv"

	// artifactsTag is the fixed, non-versioned release that hosts all FrankenPHP
	// and static PHP CLI binaries. It's rebuilt by the weekly cron / manual
	// dispatch of build-artifacts.yml and is independent of pv's own versioned
	// releases, which only ship the pv binary itself.
	artifactsTag = "artifacts"
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

	// 1. Download FrankenPHP binary for this PHP version.
	assetName, err := frankenphpAssetName(phpVersion)
	if err != nil {
		return err
	}

	fpURL := fmt.Sprintf("https://github.com/%s/releases/download/%s/%s", releaseRepo, artifactsTag, assetName)
	fpDest := FrankenPHPPath(phpVersion)

	if err := binaries.DownloadProgress(client, fpURL, fpDest, progress); err != nil {
		return fmt.Errorf("download FrankenPHP: %w", err)
	}
	if err := binaries.MakeExecutable(fpDest); err != nil {
		return err
	}

	// 2. Download PHP CLI from the same artifacts release. Built alongside
	// FrankenPHP so both binaries share an identical extension set.
	phpURL, err := phpCLIURL(artifactsTag, phpVersion)
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

	// Extract the upstream php.ini-development template if present.
	// Older artifacts (built before the per-version ini work) don't bundle
	// it; tolerate that — EnsureIniLayout handles a missing source gracefully.
	iniDevDest := filepath.Join(config.PhpEtcDir(phpVersion), "php.ini-development")
	if err := os.MkdirAll(filepath.Dir(iniDevDest), 0755); err != nil {
		return fmt.Errorf("create etc dir: %w", err)
	}
	if err := binaries.ExtractTarGz(phpArchive, iniDevDest, "php.ini-development"); err != nil {
		if !errors.Is(err, binaries.ErrEntryNotFound) {
			return fmt.Errorf("extract php.ini-development: %w", err)
		}
		// Older artifact — silently continue; EnsureIniLayout will skip the copy.
	}

	os.Remove(phpArchive)

	if err := binaries.MakeExecutable(phpDest); err != nil {
		return err
	}

	return EnsureIniLayout(phpVersion)
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
	return platformNameFor(runtime.GOOS, runtime.GOARCH)
}

func platformNameFor(goos, goarch string) (string, error) {
	archMap, ok := platformNames[goos]
	if !ok {
		return "", fmt.Errorf("unsupported OS: %s", goos)
	}
	name, ok := archMap[goarch]
	if !ok {
		return "", fmt.Errorf("unsupported architecture: %s/%s", goos, goarch)
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
