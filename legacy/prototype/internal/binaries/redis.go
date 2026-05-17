package binaries

import (
	"fmt"
	"os"
	"runtime"
)

const redisMinorVersion = "8.6"

// Redis descriptor. Single supported minor — the URL resolves to the
// artifacts-release asset for redisMinorVersion.
var Redis = Binary{
	Name:         "redis",
	DisplayName:  "Redis",
	NeedsExtract: true,
}

var redisPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "mac-arm64",
	},
}

// RedisURL returns the artifacts-release URL for redis. Today only
// darwin/arm64 is published; other platforms error.
//
// The PV_REDIS_URL_OVERRIDE environment variable, when set, replaces the
// computed URL outright. Tests use this to point installs at a local
// HTTP server. The override is applied before platform validation, so a
// test override works on any platform.
func RedisURL() (string, error) {
	if override := os.Getenv("PV_REDIS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	archMap, ok := redisPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Redis: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Redis: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/redis-%s-%s.tar.gz", platform, redisMinorVersion), nil
}
