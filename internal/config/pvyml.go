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
	PHP        string            `yaml:"php"`
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
