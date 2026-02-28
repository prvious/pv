package caddy

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"text/template"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// --- Templates for the main process (direct serving) ---

const laravelOctaneTmpl = `{{.Name}}.{{.TLD}} {
    tls internal
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        root {{.RootPath}}
        worker {
            file frankenphp-worker.php
            num 1
            watch {{.Path}}/**/*.php
        }
    }
}
`

const laravelTmpl = `{{.Name}}.{{.TLD}} {
    tls internal
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        root {{.RootPath}}
        worker index.php
    }
}
`

const phpTmpl = `{{.Name}}.{{.TLD}} {
    tls internal
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        root {{.RootPath}}
        worker index.php
    }
}
`

const staticTmpl = `{{.Name}}.{{.TLD}} {
    tls internal
    root * {{.RootPath}}
    file_server
}
`

// --- Template for reverse proxy (main process proxies to secondary) ---

const proxyTmpl = `{{.Name}}.{{.TLD}} {
    tls internal
    reverse_proxy 127.0.0.1:{{.Port}}
}
`

// --- Templates for secondary FrankenPHP instances ---

const versionLaravelOctaneTmpl = `http://{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        root {{.RootPath}}
        worker {
            file frankenphp-worker.php
            num 1
            watch {{.Path}}/**/*.php
        }
    }
}
`

const versionLaravelTmpl = `http://{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        root {{.RootPath}}
        worker index.php
    }
}
`

const versionPhpTmpl = `http://{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        root {{.RootPath}}
        worker index.php
    }
}
`

// --- Caddyfile templates ---

const mainCaddyfile = `{
    frankenphp
    local_certs
}

import sites/*
`

const versionCaddyfileTmpl = `{
    frankenphp
    auto_https off
    admin off
    http_port {{.Port}}
}

import sites-{{.Version}}/*
`

// --- Data types ---

type siteData struct {
	Name     string
	Path     string
	RootPath string
	TLD      string
	Port     int
}

type versionCaddyData struct {
	Version string
	Port    int
}

// GenerateSiteConfig generates caddy config files for a project.
// If globalPHP is empty, all PHP projects are served directly (single-version mode).
// If the project uses the globalPHP version (or is static/unknown), it generates
// a direct php_server/file_server config in sites/.
// If it uses a different PHP version, it generates a reverse_proxy in sites/ and
// a php_server config in sites-{version}/.
func GenerateSiteConfig(p registry.Project, globalPHP string) error {
	if p.Type == "" {
		return nil
	}

	if err := config.EnsureDirs(); err != nil {
		return err
	}

	settings, err := config.LoadSettings()
	if err != nil {
		return err
	}

	rootPath := resolveRoot(p)
	projectPHP := effectiveVersion(p, globalPHP)

	// Determine if this project needs proxying to a secondary instance.
	needsProxy := globalPHP != "" && projectPHP != globalPHP && isPhpType(p.Type)

	if needsProxy {
		// Write reverse_proxy config to sites/ for the main process.
		port := config.PortForVersion(projectPHP)
		if err := writeConfig(config.SitesDir(), p, settings, rootPath, proxyTmpl, port); err != nil {
			return err
		}

		// Write php_server config to sites-{version}/ for the secondary process.
		versionSitesDir := config.VersionSitesDir(projectPHP)
		if err := os.MkdirAll(versionSitesDir, 0755); err != nil {
			return err
		}
		tmplStr := versionTemplateForType(p.Type)
		return writeConfig(versionSitesDir, p, settings, rootPath, tmplStr, port)
	}

	// Direct serving by the main process.
	tmplStr := templateForType(p.Type)
	return writeConfig(config.SitesDir(), p, settings, rootPath, tmplStr, 0)
}

// RemoveSiteConfig removes all caddy configs for a project (main + version-specific).
func RemoveSiteConfig(name string) error {
	// Remove from main sites dir.
	mainPath := filepath.Join(config.SitesDir(), name+".caddy")
	if err := os.Remove(mainPath); err != nil && !os.IsNotExist(err) {
		return err
	}

	// Remove from any version-specific sites dirs.
	configDir := config.ConfigDir()
	entries, err := os.ReadDir(configDir)
	if err != nil {
		return nil // Config dir might not exist yet.
	}
	for _, e := range entries {
		if e.IsDir() && len(e.Name()) > 6 && e.Name()[:6] == "sites-" {
			vPath := filepath.Join(configDir, e.Name(), name+".caddy")
			if err := os.Remove(vPath); err != nil && !os.IsNotExist(err) {
				return err
			}
		}
	}

	return nil
}

// GenerateCaddyfile generates the main Caddyfile.
func GenerateCaddyfile() error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	return os.WriteFile(config.CaddyfilePath(), []byte(mainCaddyfile), 0644)
}

// GenerateVersionCaddyfile generates a secondary Caddyfile for a specific PHP version.
func GenerateVersionCaddyfile(version string) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	port := config.PortForVersion(version)
	tmpl, err := template.New("versionCaddyfile").Parse(versionCaddyfileTmpl)
	if err != nil {
		return err
	}

	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, versionCaddyData{Version: version, Port: port}); err != nil {
		return err
	}

	return os.WriteFile(config.VersionCaddyfilePath(version), buf.Bytes(), 0644)
}

// GenerateAllConfigs regenerates all site configs and version Caddyfiles from scratch.
// It clears existing site configs first, then regenerates from the project list.
func GenerateAllConfigs(projects []registry.Project, globalPHP string) error {
	// Clean all site config directories.
	if err := cleanSitesDirs(); err != nil {
		return err
	}

	// Generate site configs for each project.
	for _, p := range projects {
		if err := GenerateSiteConfig(p, globalPHP); err != nil {
			return err
		}
	}

	// Generate main Caddyfile.
	if err := GenerateCaddyfile(); err != nil {
		return err
	}

	// Generate version-specific Caddyfiles for secondary versions.
	active := ActiveVersions(projects, globalPHP)
	for version := range active {
		if err := GenerateVersionCaddyfile(version); err != nil {
			return err
		}
	}

	// Clean up stale version Caddyfiles.
	return cleanupStaleVersionCaddyfiles(active)
}

// ActiveVersions returns the set of non-global PHP versions that have linked projects.
func ActiveVersions(projects []registry.Project, globalPHP string) map[string]bool {
	active := make(map[string]bool)
	for _, p := range projects {
		if p.Type == "" || !isPhpType(p.Type) {
			continue
		}
		v := effectiveVersion(p, globalPHP)
		if v != globalPHP && v != "" {
			active[v] = true
		}
	}
	return active
}

// GenerateAllSiteConfigs generates site configs for all projects (single-version mode).
// This is the backward-compatible version that doesn't handle multi-version.
func GenerateAllSiteConfigs(projects []registry.Project) error {
	for _, p := range projects {
		if err := GenerateSiteConfig(p, ""); err != nil {
			return err
		}
	}
	return nil
}

// --- Helpers ---

func writeConfig(dir string, p registry.Project, settings *config.Settings, rootPath, tmplStr string, port int) error {
	tmpl, err := template.New("site").Parse(tmplStr)
	if err != nil {
		return err
	}

	var buf bytes.Buffer
	if err := tmpl.Execute(&buf, siteData{
		Name:     p.Name,
		Path:     p.Path,
		RootPath: rootPath,
		TLD:      settings.TLD,
		Port:     port,
	}); err != nil {
		return err
	}

	outPath := filepath.Join(dir, p.Name+".caddy")
	return os.WriteFile(outPath, buf.Bytes(), 0644)
}

func effectiveVersion(p registry.Project, globalPHP string) string {
	if p.PHP != "" {
		return p.PHP
	}
	return globalPHP
}

func isPhpType(t string) bool {
	switch t {
	case "laravel", "laravel-octane", "php":
		return true
	}
	return false
}

func resolveRoot(p registry.Project) string {
	switch p.Type {
	case "laravel", "laravel-octane":
		return filepath.Join(p.Path, "public")
	case "php":
		pub := filepath.Join(p.Path, "public")
		if info, err := os.Stat(pub); err == nil && info.IsDir() {
			return pub
		}
		return p.Path
	default:
		return p.Path
	}
}

func templateForType(t string) string {
	switch t {
	case "laravel-octane":
		return laravelOctaneTmpl
	case "laravel":
		return laravelTmpl
	case "php":
		return phpTmpl
	case "static":
		return staticTmpl
	default:
		return ""
	}
}

func versionTemplateForType(t string) string {
	switch t {
	case "laravel-octane":
		return versionLaravelOctaneTmpl
	case "laravel":
		return versionLaravelTmpl
	case "php":
		return versionPhpTmpl
	default:
		return ""
	}
}

func cleanSitesDirs() error {
	// Clean main sites dir.
	sitesDir := config.SitesDir()
	if err := os.RemoveAll(sitesDir); err != nil {
		return err
	}
	if err := os.MkdirAll(sitesDir, 0755); err != nil {
		return err
	}

	// Clean version-specific sites dirs.
	configDir := config.ConfigDir()
	entries, err := os.ReadDir(configDir)
	if err != nil {
		return nil
	}
	for _, e := range entries {
		if e.IsDir() && len(e.Name()) > 6 && e.Name()[:6] == "sites-" {
			if err := os.RemoveAll(filepath.Join(configDir, e.Name())); err != nil {
				return err
			}
		}
	}

	return nil
}

func cleanupStaleVersionCaddyfiles(active map[string]bool) error {
	configDir := config.ConfigDir()
	entries, err := os.ReadDir(configDir)
	if err != nil {
		return nil
	}

	for _, e := range entries {
		name := e.Name()
		if !e.IsDir() && len(name) > 4 && name[:4] == "php-" && filepath.Ext(name) == ".Caddyfile" {
			// Extract version from "php-8.3.Caddyfile".
			version := name[4 : len(name)-len(".Caddyfile")]
			if !active[version] {
				path := filepath.Join(configDir, name)
				if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
					return fmt.Errorf("cannot remove stale Caddyfile %s: %w", path, err)
				}
			}
		}
	}
	return nil
}
