package fixtures

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/project"
	"github.com/prvious/pv/test/e2e/harness"
)

// LaravelFixture describes a generated minimal Laravel project tree.
type LaravelFixture struct {
	Root string
	Name string
}

// LaravelOption customizes generated Laravel fixture metadata.
type LaravelOption func(*laravelConfig)

// ContractOption customizes the pv.yml written for link and setup scenarios.
type ContractOption func(*project.Contract)

type laravelConfig struct {
	name string
}

// WithName sets the fixture project name used by generated pv.yml hosts.
func WithName(name string) LaravelOption {
	return func(config *laravelConfig) {
		config.name = name
	}
}

// WithHosts replaces the generated pv.yml host declarations.
func WithHosts(hosts ...string) ContractOption {
	return func(contract *project.Contract) {
		contract.Hosts = append([]string(nil), hosts...)
	}
}

// WithServices replaces the generated pv.yml service declarations.
func WithServices(services ...string) ContractOption {
	return func(contract *project.Contract) {
		contract.Services = append([]string(nil), services...)
	}
}

// WithSetup replaces the generated pv.yml setup commands.
func WithSetup(commands ...string) ContractOption {
	return func(contract *project.Contract) {
		contract.Setup = append([]string(nil), commands...)
	}
}

// WithBrokenSetup configures a setup command that exits non-zero.
func WithBrokenSetup() ContractOption {
	return WithSetup("false")
}

// NewLaravel generates a deterministic minimal Laravel fixture under the sandbox project root.
func NewLaravel(sandbox *harness.Sandbox, options ...LaravelOption) (LaravelFixture, error) {
	if sandbox == nil {
		return LaravelFixture{}, fmt.Errorf("sandbox is required")
	}

	config := laravelConfig{name: "app"}
	for _, option := range options {
		option(&config)
	}
	name := strings.TrimSpace(config.name)
	if name == "" {
		name = "app"
	}

	fixture := LaravelFixture{
		Root: sandbox.ProjectRoot,
		Name: name,
	}
	files := []fixtureFile{
		{Path: "artisan", Mode: 0o755, Contents: artisanFixture},
		{Path: "composer.json", Mode: 0o644, Contents: composerFixture},
		{Path: ".env.example", Mode: 0o644, Contents: envExampleFixture},
		{Path: "bootstrap/app.php", Mode: 0o644, Contents: phpPlaceholder("bootstrap app")},
		{Path: "public/index.php", Mode: 0o644, Contents: phpPlaceholder("public index")},
		{Path: "routes/web.php", Mode: 0o644, Contents: phpPlaceholder("web routes")},
		{Path: "storage/logs/.gitkeep", Mode: 0o644, Contents: ""},
	}
	for _, file := range files {
		if err := writeFixtureFile(fixture.Root, file); err != nil {
			return LaravelFixture{}, err
		}
	}

	return fixture, nil
}

// WriteContract writes a hermetic pv.yml for link, setup, status, and helper scenarios.
func (f LaravelFixture) WriteContract(options ...ContractOption) (project.Contract, error) {
	contract := project.DefaultLaravelContract(f.Name)
	contract.Setup = nil
	for _, option := range options {
		option(&contract)
	}
	if err := contract.Validate(); err != nil {
		return project.Contract{}, err
	}
	if err := writeFileUnderRoot(f.Root, "pv.yml", []byte(contract.String()), 0o644); err != nil {
		return project.Contract{}, err
	}
	return contract, nil
}

// WriteEnv writes an existing .env fixture for declared-env merge scenarios.
func (f LaravelFixture) WriteEnv(contents string) error {
	return writeFileUnderRoot(f.Root, ".env", []byte(contents), 0o600)
}

type fixtureFile struct {
	Path     string
	Contents string
	Mode     os.FileMode
}

const artisanFixture = `#!/usr/bin/env php
<?php

fwrite(STDOUT, "fixture artisan " . implode(" ", array_slice($argv, 1)) . PHP_EOL);
`

const composerFixture = `{
  "name": "pv/e2e-laravel-fixture",
  "type": "project",
  "require": {
    "php": "^8.4",
    "laravel/framework": "^13.0"
  },
  "autoload": {
    "psr-4": {
      "App\\": "app/"
    }
  }
}
`

const envExampleFixture = `APP_NAME=Laravel
APP_ENV=local
APP_KEY=
APP_DEBUG=true
APP_URL=http://localhost
`

func phpPlaceholder(label string) string {
	return "<?php\n\n// " + label + "\n"
}

func writeFixtureFile(root string, file fixtureFile) error {
	return writeFileUnderRoot(root, file.Path, []byte(file.Contents), file.Mode)
}

func writeFileUnderRoot(root string, name string, data []byte, mode os.FileMode) error {
	path := filepath.Join(root, filepath.FromSlash(name))
	if !isWithin(root, path) {
		return fmt.Errorf("fixture path %s is outside root %s", path, root)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("create fixture directory %s: %w", filepath.Dir(path), err)
	}
	if err := os.WriteFile(path, data, mode); err != nil {
		return fmt.Errorf("write fixture file %s: %w", path, err)
	}
	return nil
}

func isWithin(root string, path string) bool {
	rel, err := filepath.Rel(root, path)
	if err != nil {
		return false
	}
	return rel == "." || (rel != ".." && !strings.HasPrefix(rel, ".."+string(os.PathSeparator)))
}
