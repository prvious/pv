package binaries

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
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

	return fetchLatestVersionFromURL(client, b.Name, url)
}

// fetchLatestVersionFromURL fetches the latest release tag from the given URL.
// When name is "rustfs", it expects the GitHub releases list endpoint (returns a
// JSON array) and picks the first entry. All other binaries expect the single
// /releases/latest object response.
func fetchLatestVersionFromURL(client *http.Client, name, url string) (string, error) {
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	SetGitHubHeaders(req)

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

	if name == "rustfs" {
		var releases []struct {
			TagName string `json:"tag_name"`
		}
		if err := json.Unmarshal(body, &releases); err != nil {
			return "", fmt.Errorf("parse GitHub response: %w", err)
		}
		if len(releases) == 0 {
			return "", fmt.Errorf("no releases found for rustfs")
		}
		return releases[0].TagName, nil
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

// SetGitHubHeaders sets standard GitHub API headers on a request,
// including Authorization from GITHUB_TOKEN if available.
func SetGitHubHeaders(req *http.Request) {
	req.Header.Set("Accept", "application/vnd.github+json")
	if token := os.Getenv("GITHUB_TOKEN"); token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
}
