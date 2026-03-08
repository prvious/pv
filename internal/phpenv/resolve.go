package phpenv

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// ResolveVersion determines the PHP version for a project directory.
// Priority: pv.yml php field → composer.json require.php → global default.
// This checks the exact directory only (used by link/server where the project path is known).
func ResolveVersion(projectPath string) (string, error) {
	// 1. Check for pv.yml file (explicit override).
	if v, err := readPvYml(projectPath); err == nil && v != "" {
		return v, nil
	}

	// 2. Check composer.json for PHP constraint.
	if v, err := resolveFromComposer(projectPath); err == nil && v != "" {
		return v, nil
	}

	// 3. Fall back to global default.
	return GlobalVersion()
}

// ResolveVersionWalkUp walks up from startDir looking for pv.yml or composer.json,
// then falls back to the global default. Used by php:current where the project
// root is unknown and must be discovered.
func ResolveVersionWalkUp(startDir string) (string, error) {
	// 1. Walk up looking for pv.yml.
	if cfg, err := config.FindAndLoadProjectConfig(startDir); err == nil && cfg != nil && cfg.PHP != "" {
		return cfg.PHP, nil
	}

	// 2. Walk up looking for composer.json.
	if v, err := walkUpComposer(startDir); err == nil && v != "" {
		return v, nil
	}

	// 3. Fall back to global default.
	return GlobalVersion()
}

// readPvYml reads the pv.yml file in the project directory and returns the PHP version.
func readPvYml(projectPath string) (string, error) {
	path := filepath.Join(projectPath, config.ProjectConfigFilename)
	cfg, err := config.LoadProjectConfig(path)
	if err != nil {
		return "", err
	}
	if cfg.PHP == "" {
		return "", fmt.Errorf("no php field in pv.yml")
	}
	return cfg.PHP, nil
}

// walkUpComposer walks up from startDir looking for a composer.json with a PHP constraint.
func walkUpComposer(startDir string) (string, error) {
	dir := startDir
	for {
		if v, err := resolveFromComposer(dir); err == nil && v != "" {
			return v, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("no composer.json found")
		}
		dir = parent
	}
}

// resolveFromComposer reads composer.json and matches the require.php constraint
// against installed versions, returning the highest satisfying version.
func resolveFromComposer(projectPath string) (string, error) {
	data, err := os.ReadFile(filepath.Join(projectPath, "composer.json"))
	if err != nil {
		return "", err
	}

	var composer struct {
		Require map[string]string `json:"require"`
	}
	if err := json.Unmarshal(data, &composer); err != nil {
		return "", err
	}

	constraint, ok := composer.Require["php"]
	if !ok || constraint == "" {
		return "", fmt.Errorf("no php requirement in composer.json")
	}

	installed, err := InstalledVersions()
	if err != nil {
		return "", err
	}
	if len(installed) == 0 {
		return "", fmt.Errorf("no PHP versions installed")
	}

	matched := matchConstraint(constraint, installed)
	if matched == "" {
		return "", fmt.Errorf("no installed PHP version satisfies %q", constraint)
	}
	return matched, nil
}

// matchConstraint returns the highest installed version that satisfies
// a Composer-style PHP constraint. Returns "" if none match.
//
// Supported constraint formats:
//   - ^8.2     → >=8.2, <9.0
//   - ~8.2     → >=8.2, <9.0
//   - ~8.2.0   → >=8.2, <8.3
//   - >=8.2    → >=8.2
//   - 8.2.*    → 8.2
//   - >=8.2 <8.5 → >=8.2, <8.5
//   - 8.2|8.3  → 8.2 or 8.3
func matchConstraint(constraint string, installed []string) string {
	// Handle OR constraints (e.g., "8.2|8.3" or "^8.2 || ^8.3").
	constraint = strings.ReplaceAll(constraint, "||", "|")
	parts := strings.Split(constraint, "|")

	var best string
	for _, part := range parts {
		part = strings.TrimSpace(part)
		if v := matchSingleConstraint(part, installed); v != "" {
			if best == "" || compareVersions(v, best) > 0 {
				best = v
			}
		}
	}
	return best
}

var (
	reCaretTilde = regexp.MustCompile(`^([~^])(\d+\.\d+)(?:\.\d+)?$`)
	reGtEqLt     = regexp.MustCompile(`^>=\s*(\d+\.\d+)(?:\.\d+)?\s+<\s*(\d+\.\d+)`)
	reGtEq       = regexp.MustCompile(`^>=\s*(\d+\.\d+)`)
	reWildcard   = regexp.MustCompile(`^(\d+\.\d+)\.\*$`)
	reExact      = regexp.MustCompile(`^(\d+\.\d+)(?:\.\d+)?$`)
)

func matchSingleConstraint(constraint string, installed []string) string {
	constraint = strings.TrimSpace(constraint)

	// >=8.2 <8.5 (range)
	if m := reGtEqLt.FindStringSubmatch(constraint); m != nil {
		return highestInRange(installed, m[1], m[2])
	}

	// ^8.2 or ~8.2 (caret/tilde on major.minor)
	if m := reCaretTilde.FindStringSubmatch(constraint); m != nil {
		minV := m[2]
		major := strings.Split(minV, ".")[0]
		nextMajor := fmt.Sprintf("%d.0", atoi(major)+1)
		return highestInRange(installed, minV, nextMajor)
	}

	// >=8.2
	if m := reGtEq.FindStringSubmatch(constraint); m != nil {
		return highestAtLeast(installed, m[1])
	}

	// 8.2.*
	if m := reWildcard.FindStringSubmatch(constraint); m != nil {
		return exactMatch(installed, m[1])
	}

	// 8.2 or 8.2.1 (exact)
	if m := reExact.FindStringSubmatch(constraint); m != nil {
		return exactMatch(installed, m[1])
	}

	// Fallback: try to extract a version number and match it.
	re := regexp.MustCompile(`(\d+\.\d+)`)
	if m := re.FindStringSubmatch(constraint); m != nil {
		return highestAtLeast(installed, m[1])
	}

	return ""
}

// highestInRange returns the highest installed version >= min and < max.
func highestInRange(installed []string, min, max string) string {
	var best string
	for _, v := range installed {
		if compareVersions(v, min) >= 0 && compareVersions(v, max) < 0 {
			if best == "" || compareVersions(v, best) > 0 {
				best = v
			}
		}
	}
	return best
}

// highestAtLeast returns the highest installed version >= min.
func highestAtLeast(installed []string, min string) string {
	var best string
	for _, v := range installed {
		if compareVersions(v, min) >= 0 {
			if best == "" || compareVersions(v, best) > 0 {
				best = v
			}
		}
	}
	return best
}

// exactMatch returns the version if it exists in installed.
func exactMatch(installed []string, version string) string {
	for _, v := range installed {
		if v == version {
			return v
		}
	}
	return ""
}

func atoi(s string) int {
	var n int
	fmt.Sscanf(s, "%d", &n)
	return n
}
