package binaries

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/prvious/pv/internal/config"
)

type VersionState struct {
	Versions map[string]string `json:"versions"`
}

func LoadVersions() (*VersionState, error) {
	path := config.VersionsPath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &VersionState{Versions: make(map[string]string)}, nil
		}
		return nil, err
	}
	var vs VersionState
	if err := json.Unmarshal(data, &vs); err != nil {
		return nil, err
	}
	if vs.Versions == nil {
		vs.Versions = make(map[string]string)
	}
	return &vs, nil
}

func (vs *VersionState) Save() error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(vs, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(config.VersionsPath(), data, 0644)
}

func (vs *VersionState) Get(name string) string {
	return vs.Versions[name]
}

func (vs *VersionState) Set(name, version string) {
	vs.Versions[name] = version
}

// FetchLatestVersion queries GitHub API for the latest release tag.
// For Composer, it returns "latest" (always re-downloaded).
func FetchLatestVersion(client *http.Client, b Binary) (string, error) {
	if b.Name == "composer" {
		return "latest", nil
	}

	url := LatestVersionURL(b)
	if url == "" {
		return "", fmt.Errorf("no version URL for %s", b.Name)
	}

	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("Accept", "application/vnd.github+json")

	resp, err := client.Do(req)
	if err != nil {
		return "", fmt.Errorf("fetch latest version failed: %w", err)
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
		return "", fmt.Errorf("parse GitHub response: %w", err)
	}

	return release.TagName, nil
}

// NeedsUpdate returns true if the binary needs to be updated.
// It normalizes version strings by stripping a leading "v" prefix for comparison.
func NeedsUpdate(vs *VersionState, b Binary, latestVersion string) bool {
	installed := vs.Get(b.Name)
	if installed == "" {
		return true
	}
	return normalizeVersion(installed) != normalizeVersion(latestVersion)
}

func normalizeVersion(v string) string {
	return strings.TrimPrefix(v, "v")
}

// ParseFrankenPHPPhpVersion extracts the PHP version from `frankenphp version` output.
// Example input: "FrankenPHP v1.11.3 PHP 8.5.3 Caddy/v2.9.1 h1:..."
// Returns: "8.5.3"
func ParseFrankenPHPPhpVersion(output string) (string, error) {
	re := regexp.MustCompile(`PHP (\d+\.\d+\.\d+)`)
	matches := re.FindStringSubmatch(output)
	if len(matches) < 2 {
		return "", fmt.Errorf("could not parse PHP version from FrankenPHP output: %s", output)
	}
	return matches[1], nil
}

// DetectPHPVersion runs `frankenphp version` and parses the embedded PHP version.
func DetectPHPVersion(binDir string) (string, error) {
	frankenphpPath := filepath.Join(binDir, "frankenphp")
	out, err := exec.Command(frankenphpPath, "version").Output()
	if err != nil {
		return "", fmt.Errorf("run frankenphp version: %w", err)
	}
	return ParseFrankenPHPPhpVersion(string(out))
}
