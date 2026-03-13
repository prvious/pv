package packages

import (
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

// PharPath returns the full path where this package's PHAR is stored.
func (p Package) PharPath() string {
	return filepath.Join(config.PackagesDir(), p.Name+".phar")
}

// SymlinkPath returns the full path for the symlink in the user's PATH.
func (p Package) SymlinkPath() string {
	return filepath.Join(config.BinDir(), p.Name)
}

// LatestReleaseURL returns the GitHub API URL for the latest release.
func (p Package) LatestReleaseURL() string {
	return "https://api.github.com/repos/" + p.Repo + "/releases/latest"
}
