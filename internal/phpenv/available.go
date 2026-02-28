package phpenv

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"sort"
)

// AvailableVersions fetches the PHP versions available in the latest
// prvious/pv release by examining asset names.
func AvailableVersions(client *http.Client) ([]string, error) {
	tag, err := latestReleaseTag(client)
	if err != nil {
		return nil, err
	}

	url := fmt.Sprintf("https://api.github.com/repos/%s/releases/tags/%s", releaseRepo, tag)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/vnd.github+json")

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("GitHub API returned HTTP %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	var release struct {
		Assets []struct {
			Name string `json:"name"`
		} `json:"assets"`
	}
	if err := json.Unmarshal(body, &release); err != nil {
		return nil, err
	}

	// Extract PHP versions from asset names like "frankenphp-mac-arm64-php8.4".
	re := regexp.MustCompile(`-php(\d+\.\d+)$`)
	seen := make(map[string]bool)
	for _, a := range release.Assets {
		m := re.FindStringSubmatch(a.Name)
		if m != nil {
			seen[m[1]] = true
		}
	}

	var versions []string
	for v := range seen {
		versions = append(versions, v)
	}
	sort.Slice(versions, func(i, j int) bool {
		return compareVersions(versions[i], versions[j]) < 0
	})

	return versions, nil
}
