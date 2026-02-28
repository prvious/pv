package config

import (
	"fmt"
	"os"
	"path/filepath"
)

func PvDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".pv")
}

func ConfigDir() string {
	return filepath.Join(PvDir(), "config")
}

func SitesDir() string {
	return filepath.Join(ConfigDir(), "sites")
}

func LogsDir() string {
	return filepath.Join(PvDir(), "logs")
}

func DataDir() string {
	return filepath.Join(PvDir(), "data")
}

func BinDir() string {
	return filepath.Join(PvDir(), "bin")
}

func RegistryPath() string {
	return filepath.Join(DataDir(), "registry.json")
}

func PidFilePath() string {
	return filepath.Join(DataDir(), "pv.pid")
}

func CaddyLogPath() string {
	return filepath.Join(LogsDir(), "caddy.log")
}

func CaddyLogPathForVersion(version string) string {
	return filepath.Join(LogsDir(), "caddy-"+version+".log")
}

const DNSPort = 10053

func PhpDir() string {
	return filepath.Join(PvDir(), "php")
}

func PhpVersionDir(version string) string {
	return filepath.Join(PhpDir(), version)
}

func VersionSitesDir(version string) string {
	return filepath.Join(ConfigDir(), "sites-"+version)
}

func VersionCaddyfilePath(version string) string {
	return filepath.Join(ConfigDir(), "php-"+version+".Caddyfile")
}

// PortForVersion returns the HTTP port for a secondary FrankenPHP instance.
// Scheme: 8000 + major*100 + minor*10, e.g. PHP 8.3 → 8830, PHP 8.4 → 8840.
func PortForVersion(version string) int {
	var major, minor int
	fmt.Sscanf(version, "%d.%d", &major, &minor)
	return 8000 + major*100 + minor*10
}

func VersionsPath() string {
	return filepath.Join(DataDir(), "versions.json")
}

func SettingsPath() string {
	return filepath.Join(ConfigDir(), "settings.json")
}

func CaddyfilePath() string {
	return filepath.Join(ConfigDir(), "Caddyfile")
}

// CaddyEnv returns environment variable strings that direct Caddy to store
// data under ~/.pv/caddy/ instead of platform-default directories.
func CaddyEnv() []string {
	pvDir := PvDir()
	return []string{
		"XDG_DATA_HOME=" + pvDir,
		"XDG_CONFIG_HOME=" + pvDir,
	}
}

// CACertPath returns the path to Caddy's local CA root certificate.
func CACertPath() string {
	return filepath.Join(PvDir(), "caddy", "pki", "authorities", "local", "root.crt")
}

func EnsureDirs() error {
	dirs := []string{
		ConfigDir(),
		SitesDir(),
		LogsDir(),
		DataDir(),
		BinDir(),
		PhpDir(),
	}
	for _, dir := range dirs {
		if err := os.MkdirAll(dir, 0755); err != nil {
			return err
		}
	}
	return nil
}
