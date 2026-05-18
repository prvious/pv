package postgres

import (
	"os"
	"path/filepath"
	"sort"

	"github.com/prvious/pv/internal/config"
)

// InstalledMajors returns the sorted list of postgres majors that have a
// runnable bin/postgres on disk. A directory under ~/.pv/postgres/ with no
// bin/postgres is treated as not-installed (incomplete extraction, etc.).
func InstalledMajors() ([]string, error) {
	root := config.PostgresDir()
	entries, err := os.ReadDir(root)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var out []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		major := e.Name()
		bin := filepath.Join(config.PostgresBinDir(major), "postgres")
		if info, err := os.Stat(bin); err == nil && !info.IsDir() {
			out = append(out, major)
		}
	}
	sort.Strings(out)
	return out, nil
}

// IsInstalled is a convenience wrapper for callers that want a yes/no.
func IsInstalled(major string) bool {
	bin := filepath.Join(config.PostgresBinDir(major), "postgres")
	info, err := os.Stat(bin)
	return err == nil && !info.IsDir()
}
