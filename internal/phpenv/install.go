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
	// releaseRepo is the GitHub repository hosting custom FrankenPHP builds.
	releaseRepo = "prvious/pv"
)

// Install downloads and installs a PHP version (FrankenPHP + PHP CLI).
// The phpVersion is a major.minor string like "8.4".
func Install(client *http.Client, phpVersion string) error {
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

	fmt.Printf("  Downloading FrankenPHP (PHP %s)...\n", phpVersion)
	if err := binaries.Download(client, fpURL, fpDest); err != nil {
		return fmt.Errorf("download FrankenPHP: %w", err)
	}
	if err := binaries.MakeExecutable(fpDest); err != nil {
		return err
	}

	// 3. Detect the full PHP version from the binary.
	fullVersion, err := binaries.DetectPHPVersion(versionDir)
	if err != nil {
		fmt.Printf("  (could not detect full PHP version: %v)\n", err)
		fullVersion = phpVersion + ".0"
	}

	// 4. Download PHP CLI from static-php.dev.
	phpURL, err := phpCLIURL(fullVersion)
	if err != nil {
		return err
	}

	phpArchive := fpDest + ".php.tar.gz"
	phpDest := PHPPath(phpVersion)

	fmt.Printf("  Downloading PHP CLI %s...\n", fullVersion)
	if err := binaries.Download(client, phpURL, phpArchive); err != nil {
		return fmt.Errorf("download PHP CLI: %w", err)
	}

	if err := binaries.ExtractTarGz(phpArchive, phpDest, "php"); err != nil {
		return fmt.Errorf("extract PHP CLI: %w", err)
	}
	os.Remove(phpArchive)

	if err := binaries.MakeExecutable(phpDest); err != nil {
		return err
	}

	fmt.Printf("  ✓ PHP %s installed\n", phpVersion)
	return nil
}

// latestReleaseTag fetches the latest release tag from the prvious/pv repo.
func latestReleaseTag(client *http.Client) (string, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/releases/latest", releaseRepo)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("Accept", "application/vnd.github+json")

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

var phpArchNames = map[string]string{
	"arm64": "aarch64",
	"amd64": "x86_64",
}

var phpOSNames = map[string]string{
	"darwin": "macos",
	"linux":  "linux",
}

func phpCLIURL(fullVersion string) (string, error) {
	arch, ok := phpArchNames[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for PHP CLI: %s", runtime.GOARCH)
	}
	osName, ok := phpOSNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for PHP CLI: %s", runtime.GOOS)
	}
	return fmt.Sprintf("https://dl.static-php.dev/static-php-cli/common/php-%s-cli-%s-%s.tar.gz", fullVersion, osName, arch), nil
}
