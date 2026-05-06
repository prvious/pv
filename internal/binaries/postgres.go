package binaries

import (
	"fmt"
	"runtime"
)

var Postgres = Binary{
	Name:         "postgres",
	DisplayName:  "PostgreSQL",
	NeedsExtract: true,
}

var postgresPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "mac-arm64",
	},
}

func PostgresURL(major string) (string, error) {
	archMap, ok := postgresPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for PostgreSQL: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for PostgreSQL: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/postgres-%s-%s.tar.gz", platform, major), nil
}
