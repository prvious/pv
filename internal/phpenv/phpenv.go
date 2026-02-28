package phpenv

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// InstalledVersions returns all PHP versions that have been installed.
// It scans ~/.pv/php/ for directories containing a frankenphp binary.
func InstalledVersions() ([]string, error) {
	phpDir := config.PhpDir()
	entries, err := os.ReadDir(phpDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}

	var versions []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		// Verify the directory contains a frankenphp binary.
		fp := filepath.Join(phpDir, e.Name(), "frankenphp")
		if _, err := os.Stat(fp); err == nil {
			versions = append(versions, e.Name())
		}
	}

	sort.Slice(versions, func(i, j int) bool {
		return compareVersions(versions[i], versions[j]) < 0
	})

	return versions, nil
}

// IsInstalled returns true if the given PHP version is installed.
func IsInstalled(version string) bool {
	fp := filepath.Join(config.PhpVersionDir(version), "frankenphp")
	_, err := os.Stat(fp)
	return err == nil
}

// FrankenPHPPath returns the path to the FrankenPHP binary for a version.
func FrankenPHPPath(version string) string {
	return filepath.Join(config.PhpVersionDir(version), "frankenphp")
}

// PHPPath returns the path to the PHP CLI binary for a version.
func PHPPath(version string) string {
	return filepath.Join(config.PhpVersionDir(version), "php")
}

// SetGlobal updates the global PHP version in settings and repoints symlinks.
func SetGlobal(version string) error {
	if !IsInstalled(version) {
		return fmt.Errorf("PHP %s is not installed", version)
	}

	settings, err := config.LoadSettings()
	if err != nil {
		return err
	}
	settings.GlobalPHP = version
	if err := settings.Save(); err != nil {
		return err
	}

	return updateSymlinks(version)
}

// GlobalVersion returns the currently configured global PHP version.
func GlobalVersion() (string, error) {
	settings, err := config.LoadSettings()
	if err != nil {
		return "", err
	}
	if settings.GlobalPHP == "" {
		return "", fmt.Errorf("no global PHP version set (run: pv php install <version>)")
	}
	return settings.GlobalPHP, nil
}

// Remove deletes an installed PHP version.
func Remove(version string) error {
	if !IsInstalled(version) {
		return fmt.Errorf("PHP %s is not installed", version)
	}

	// Check if it's the global default.
	settings, err := config.LoadSettings()
	if err != nil {
		return err
	}
	if settings.GlobalPHP == version {
		return fmt.Errorf("cannot remove PHP %s: it is the global default (switch with: pv use php:<other-version>)", version)
	}

	return os.RemoveAll(config.PhpVersionDir(version))
}

// updateSymlinks repoints ~/.pv/bin/frankenphp to the given version.
// PHP CLI is handled by the shim script from WriteShims(), not a symlink.
func updateSymlinks(version string) error {
	binDir := config.BinDir()
	linkPath := filepath.Join(binDir, "frankenphp")
	target := FrankenPHPPath(version)
	// Remove existing file/symlink.
	os.Remove(linkPath)
	if err := os.Symlink(target, linkPath); err != nil {
		return fmt.Errorf("cannot create symlink %s → %s: %w", linkPath, target, err)
	}
	return nil
}

// compareVersions compares two version strings like "8.3" and "8.4".
// Returns negative if a < b, zero if equal, positive if a > b.
func compareVersions(a, b string) int {
	aParts := strings.Split(a, ".")
	bParts := strings.Split(b, ".")

	for i := 0; i < len(aParts) && i < len(bParts); i++ {
		av, bv := 0, 0
		fmt.Sscanf(aParts[i], "%d", &av)
		fmt.Sscanf(bParts[i], "%d", &bv)
		if av != bv {
			return av - bv
		}
	}
	return len(aParts) - len(bParts)
}
