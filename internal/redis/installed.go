package redis

import (
	"os"
	"path/filepath"
	"sort"

	"github.com/prvious/pv/internal/config"
)

func ServerBinary(version string) string {
	return filepath.Join(config.RedisVersionDir(version), "redis-server")
}

func CLIBinary(version string) string {
	return filepath.Join(config.RedisVersionDir(version), "redis-cli")
}

func IsInstalled(version string) bool {
	info, err := os.Stat(ServerBinary(version))
	return err == nil && !info.IsDir()
}

func InstalledVersions() ([]string, error) {
	root := config.RedisDir()
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
		bin := filepath.Join(config.RedisVersionDir(version), "redis-server")
		if info, err := os.Stat(bin); err == nil && !info.IsDir() {
			out = append(out, version)
		}
	}
	sort.Strings(out)
	return out, nil
}
