package caddy

import (
	"bytes"
	"os"
	"path/filepath"
	"text/template"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

const laravelOctaneTmpl = `{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    encode zstd gzip

    php_server {
        worker {
            file {{.RootPath}}/frankenphp-worker.php
            num 1
            watch {{.Path}}/**/*.php
        }
    }
}
`

const laravelTmpl = `{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    encode zstd gzip

    php_server
}
`

const phpTmpl = `{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    encode zstd gzip

    php_server
}
`

const staticTmpl = `{{.Name}}.{{.TLD}} {
    root * {{.RootPath}}
    file_server
}
`

const mainCaddyfile = `{
    frankenphp
}

import sites/*
`

type siteData struct {
	Name     string
	Path     string
	RootPath string
	TLD      string
}

func GenerateSiteConfig(p registry.Project) error {
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
	tmplStr := templateForType(p.Type)

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
	}); err != nil {
		return err
	}

	outPath := filepath.Join(config.SitesDir(), p.Name+".caddy")
	return os.WriteFile(outPath, buf.Bytes(), 0644)
}

func RemoveSiteConfig(name string) error {
	path := filepath.Join(config.SitesDir(), name+".caddy")
	if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}

func GenerateCaddyfile() error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	return os.WriteFile(config.CaddyfilePath(), []byte(mainCaddyfile), 0644)
}

func GenerateAllSiteConfigs(projects []registry.Project) error {
	for _, p := range projects {
		if err := GenerateSiteConfig(p); err != nil {
			return err
		}
	}
	return nil
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
