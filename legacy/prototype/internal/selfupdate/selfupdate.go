package selfupdate

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/prvious/pv/internal/binaries"
)

// NeedsUpdate checks if a newer pv version is available.
// Returns the latest version tag (without "v" prefix) and whether an update is needed.
func NeedsUpdate(client *http.Client, currentVersion string) (string, bool, error) {
	latest, err := fetchLatestVersion(client)
	if err != nil {
		return "", false, err
	}

	latestNorm := strings.TrimPrefix(latest, "v")
	currentNorm := strings.TrimPrefix(currentVersion, "v")

	if currentNorm == "dev" || currentNorm == "" {
		return latestNorm, false, nil
	}

	return latestNorm, latestNorm != currentNorm, nil
}

// Update downloads the latest pv binary and replaces the current one.
// Returns the path to the new binary.
func Update(client *http.Client, version string, progress func(written, total int64)) (string, error) {
	execPath, err := os.Executable()
	if err != nil {
		return "", fmt.Errorf("cannot determine executable path: %w", err)
	}
	execPath, err = filepath.EvalSymlinks(execPath)
	if err != nil {
		return "", fmt.Errorf("cannot resolve executable path: %w", err)
	}

	// Get current file permissions.
	info, err := os.Stat(execPath)
	if err != nil {
		return "", fmt.Errorf("cannot stat current binary: %w", err)
	}

	url := downloadURL(version)

	// Download to temp file in the same directory (ensures same filesystem for rename).
	tmpFile := execPath + ".tmp"
	defer os.Remove(tmpFile)

	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	binaries.SetGitHubHeaders(req)

	resp, err := client.Do(req)
	if err != nil {
		return "", fmt.Errorf("cannot download pv: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("cannot download pv: HTTP %d", resp.StatusCode)
	}

	f, err := os.OpenFile(tmpFile, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, info.Mode())
	if err != nil {
		return "", fmt.Errorf("cannot create temp file: %w", err)
	}

	var reader io.Reader = resp.Body
	if progress != nil {
		reader = &progressReader{r: resp.Body, total: resp.ContentLength, progress: progress}
	}

	if _, err := io.Copy(f, reader); err != nil {
		f.Close()
		return "", fmt.Errorf("cannot write binary: %w", err)
	}
	f.Close()

	// Atomic replace.
	if err := os.Rename(tmpFile, execPath); err != nil {
		return "", fmt.Errorf("cannot replace binary: %w", err)
	}

	return execPath, nil
}

// githubAPIURL is the base URL for GitHub API calls. Overridable in tests.
var githubAPIURL = "https://api.github.com/repos/prvious/pv/releases/"

func fetchLatestVersion(client *http.Client) (string, error) {
	url := githubAPIURL + "latest"
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	binaries.SetGitHubHeaders(req)

	resp, err := client.Do(req)
	if err != nil {
		return "", fmt.Errorf("cannot check pv version: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("GitHub API returned HTTP %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("cannot read GitHub API response: %w", err)
	}

	var release struct {
		TagName string `json:"tag_name"`
	}
	if err := json.Unmarshal(body, &release); err != nil {
		return "", fmt.Errorf("cannot parse GitHub response: %w", err)
	}

	return release.TagName, nil
}

func platformString() string {
	return fmt.Sprintf("%s-%s", runtime.GOOS, runtime.GOARCH)
}

func downloadURL(version string) string {
	version = strings.TrimPrefix(version, "v")
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/v%s/pv-%s", version, platformString())
}

type progressReader struct {
	r        io.Reader
	total    int64
	written  int64
	progress func(written, total int64)
}

func (pr *progressReader) Read(p []byte) (int, error) {
	n, err := pr.r.Read(p)
	pr.written += int64(n)
	if pr.progress != nil {
		pr.progress(pr.written, pr.total)
	}
	return n, err
}
