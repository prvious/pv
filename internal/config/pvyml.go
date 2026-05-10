package config

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

const ProjectConfigFilename = "pv.yml"

// ProjectConfig represents the contents of a pv.yml file.
type ProjectConfig struct {
	PHP string `yaml:"php"`
	// Aliases are additional hostnames Caddy should serve for this
	// project, each minted its own TLS cert. The site's `*.{name}.{tld}`
	// wildcard SAN already covers same-domain hostnames, so the typical
	// use case is foreign domains (e.g., "api.test", "dashboard.example.com").
	Aliases    []string          `yaml:"aliases,omitempty"`
	Env        map[string]string `yaml:"env,omitempty"`
	Postgresql *ServiceConfig    `yaml:"postgresql,omitempty"`
	Mysql      *ServiceConfig    `yaml:"mysql,omitempty"`
	Redis      *ServiceConfig    `yaml:"redis,omitempty"`
	Mailpit    *ServiceConfig    `yaml:"mailpit,omitempty"`
	Rustfs     *ServiceConfig    `yaml:"rustfs,omitempty"`
	Setup      []string          `yaml:"setup,omitempty"`
}

// ServiceConfig declares a backing service a project depends on.
// Version is required for postgresql and mysql (multi-version aware);
// optional and ignored for redis, mailpit, rustfs (single bundled version).
type ServiceConfig struct {
	Version string            `yaml:"version,omitempty"`
	Env     map[string]string `yaml:"env,omitempty"`
}

// LoadProjectConfig reads and parses a pv.yml file at the given path.
func LoadProjectConfig(path string) (*ProjectConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var cfg ProjectConfig
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("invalid pv.yml: %w", err)
	}
	return &cfg, nil
}

// FindProjectConfig walks up from startDir looking for a pv.yml file.
// Returns the full path to the file, or empty string if not found.
func FindProjectConfig(startDir string) string {
	dir := startDir
	for {
		path := filepath.Join(dir, ProjectConfigFilename)
		if _, err := os.Stat(path); err == nil {
			return path
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return ""
		}
		dir = parent
	}
}

// FindAndLoadProjectConfig walks up from startDir, finds pv.yml, and parses it.
// Returns nil config (no error) if no pv.yml is found.
func FindAndLoadProjectConfig(startDir string) (*ProjectConfig, error) {
	path := FindProjectConfig(startDir)
	if path == "" {
		return nil, nil
	}
	return LoadProjectConfig(path)
}

// HasServices reports whether any service block is declared in pv.yml.
// Nil-safe so it can be called on a freshly-loaded *ProjectConfig that
// may not exist for the project.
func (p *ProjectConfig) HasServices() bool {
	if p == nil {
		return false
	}
	return p.Postgresql != nil || p.Mysql != nil || p.Redis != nil ||
		p.Mailpit != nil || p.Rustfs != nil
}

// HasAnyEnv reports whether pv.yml declares any env keys — either the
// top-level Env map or any service's Env map. Used to decide whether
// the new pv.yml-driven env writer runs and the legacy Laravel
// writer skips.
func (p *ProjectConfig) HasAnyEnv() bool {
	if p == nil {
		return false
	}
	if len(p.Env) > 0 {
		return true
	}
	for _, svc := range []*ServiceConfig{p.Postgresql, p.Mysql, p.Redis, p.Mailpit, p.Rustfs} {
		if svc != nil && len(svc.Env) > 0 {
			return true
		}
	}
	return false
}
