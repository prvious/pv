package config

import (
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

const DNSPort = 10053

func VersionsPath() string {
	return filepath.Join(DataDir(), "versions.json")
}

func SettingsPath() string {
	return filepath.Join(ConfigDir(), "settings.json")
}

func CaddyfilePath() string {
	return filepath.Join(ConfigDir(), "Caddyfile")
}

func EnsureDirs() error {
	dirs := []string{
		ConfigDir(),
		SitesDir(),
		LogsDir(),
		DataDir(),
		BinDir(),
	}
	for _, dir := range dirs {
		if err := os.MkdirAll(dir, 0755); err != nil {
			return err
		}
	}
	return nil
}
