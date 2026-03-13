package packages

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// InstallMethod determines how a package is installed.
type InstallMethod int

const (
	// MethodPHAR downloads a standalone PHAR from GitHub releases.
	MethodPHAR InstallMethod = iota
	// MethodComposer installs via composer global require.
	MethodComposer
)

// Package defines a managed package that pv keeps up-to-date.
type Package struct {
	Name     string        // binary name (e.g., "laravel", "phpstan")
	Repo     string        // GitHub owner/repo (e.g., "laravel/installer")
	Asset    string        // release asset filename (MethodPHAR only)
	Method   InstallMethod // how to install this package
	Composer string        // composer package name (MethodComposer only)
}

// Managed is the compiled-in registry of packages pv manages.
var Managed = []Package{
	{Name: "laravel", Repo: "laravel/installer", Method: MethodComposer, Composer: "laravel/installer"},
}

func init() {
	seen := make(map[string]bool)
	for _, pkg := range Managed {
		if err := pkg.Validate(); err != nil {
			panic(fmt.Sprintf("invalid managed package: %v", err))
		}
		if seen[pkg.Name] {
			panic(fmt.Sprintf("duplicate managed package name: %q", pkg.Name))
		}
		seen[pkg.Name] = true
	}
}

// Validate checks that a Package has all required fields for its install method.
func (p Package) Validate() error {
	if p.Name == "" {
		return fmt.Errorf("package name is required")
	}
	switch p.Method {
	case MethodPHAR:
		if p.Repo == "" || p.Asset == "" {
			return fmt.Errorf("PHAR package %q requires Repo and Asset", p.Name)
		}
	case MethodComposer:
		if p.Composer == "" {
			return fmt.Errorf("Composer package %q requires Composer field", p.Name)
		}
	default:
		return fmt.Errorf("unknown install method %d for %q", p.Method, p.Name)
	}
	return nil
}

// PharPath returns the full path where this package's PHAR is stored (MethodPHAR only).
func (p Package) PharPath() string {
	return filepath.Join(config.PackagesDir(), p.Name+".phar")
}

// SymlinkPath returns the full path for the symlink in the user's PATH (MethodPHAR only).
func (p Package) SymlinkPath() string {
	return filepath.Join(config.BinDir(), p.Name)
}

// LatestReleaseURL returns the GitHub API URL for the latest release.
func (p Package) LatestReleaseURL() string {
	return "https://api.github.com/repos/" + p.Repo + "/releases/latest"
}
