package binaries

import (
	"fmt"
	"os"
	"runtime"
)

// Mysql descriptor. Versioned by major.minor; URL is per-version because the
// artifacts release is rolling (always carries the latest GA patch of a
// major.minor line).
var Mysql = Binary{
	Name:         "mysql",
	DisplayName:  "MySQL",
	NeedsExtract: true,
}

// supportedMysqlVersions enumerates the major.minor lines pv ships
// artifacts for. Adding a new minor (e.g. "9.8") requires an
// artifacts-pipeline update first; this list is the consumer-side
// allow-list.
var supportedMysqlVersions = map[string]struct{}{
	"8.0": {},
	"8.4": {},
	"9.7": {},
}

var mysqlPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "mac-arm64",
	},
}

// IsValidMysqlVersion reports whether the given version string is one of
// the supported major.minor lines.
func IsValidMysqlVersion(version string) bool {
	_, ok := supportedMysqlVersions[version]
	return ok
}

// MysqlURL returns the artifacts-release URL for the given major.minor.
// Today only darwin/arm64 is published; other platforms error.
//
// The PV_MYSQL_URL_OVERRIDE environment variable, when set, replaces the
// computed URL outright. Tests use this to point installs at a local
// HTTP server. The override is applied before platform/version
// validation, so a test override works on any platform.
func MysqlURL(version string) (string, error) {
	if override := os.Getenv("PV_MYSQL_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	if !IsValidMysqlVersion(version) {
		return "", fmt.Errorf("unsupported MySQL version %q (want one of 8.0, 8.4, 9.7)", version)
	}
	archMap, ok := mysqlPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for MySQL: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for MySQL: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/mysql-%s-%s.tar.gz", platform, version), nil
}
