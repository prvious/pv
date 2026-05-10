package redis

import (
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// ServerBinary returns the absolute path to the bundled redis-server.
// Used by callers that need the path (e.g. process.BuildSupervisorProcess);
// keeps the join in one place.
func ServerBinary() string {
	return filepath.Join(config.RedisDir(), "redis-server")
}

// CLIBinary returns the absolute path to the bundled redis-cli.
// Not on PATH — internal use only (e2e tests, debugging).
func CLIBinary() string {
	return filepath.Join(config.RedisDir(), "redis-cli")
}

// IsInstalled reports whether redis-server exists at the expected path.
// A directory at config.RedisDir() with no redis-server is treated as
// not-installed (incomplete extraction, etc.).
func IsInstalled() bool {
	info, err := os.Stat(ServerBinary())
	return err == nil && !info.IsDir()
}
