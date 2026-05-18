package mysql

import (
	"os"
	"path/filepath"
	"sort"

	"github.com/prvious/pv/internal/config"
)

// InstalledVersions returns the sorted list of mysql versions that have a
// runnable bin/mysqld on disk. A directory under ~/.pv/mysql/ with no
// bin/mysqld is treated as not-installed (incomplete extraction, etc.).
func InstalledVersions() ([]string, error) {
	root := config.MysqlDir()
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
		version := e.Name()
		bin := filepath.Join(config.MysqlBinDir(version), "mysqld")
		if info, err := os.Stat(bin); err == nil && !info.IsDir() {
			out = append(out, version)
		}
	}
	sort.Strings(out)
	return out, nil
}

// IsInstalled is a convenience wrapper for callers that want a yes/no.
func IsInstalled(version string) bool {
	bin := filepath.Join(config.MysqlBinDir(version), "mysqld")
	info, err := os.Stat(bin)
	return err == nil && !info.IsDir()
}
