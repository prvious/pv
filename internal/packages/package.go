package packages

import (
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// Package defines a managed PHAR package that pv keeps up-to-date.
type Package struct {
	Name  string // binary name and symlink name (e.g., "laravel")
	Repo  string // GitHub owner/repo (e.g., "laravel/installer")
	Asset string // release asset filename (e.g., "laravel.phar")
}

// Managed is the compiled-in registry of packages pv manages.
var Managed = []Package{
	{Name: "laravel", Repo: "laravel/installer", Asset: "laravel.phar"},
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
