package steps

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// stagePostgresBinary writes a stub postgres at ~/.pv/postgres/<major>/bin/postgres
// so postgres.IsInstalled(major) returns true.
func stagePostgresBinary(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", bin, err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage postgres: %v", err)
	}
}

func TestApplyPvYmlServices_BindsPostgresFromConfig(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stagePostgresBinary(t, "18")

	projDir := t.TempDir()
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectType: "laravel",
		Registry:    reg,
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{Version: "18"},
		},
	}

	step := &ApplyPvYmlServicesStep{}
	if !step.ShouldRun(ctx) {
		t.Fatal("ShouldRun: want true when pv.yml declares services")
	}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services == nil || reg.Projects[0].Services.Postgres != "18" {
		t.Errorf("Postgres binding = %+v, want major=18", reg.Projects[0].Services)
	}
}

func TestApplyPvYmlServices_ErrorsWhenVersionNotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// Do NOT stage any postgres binary.

	projDir := t.TempDir()
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		Registry:    reg,
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{Version: "18"},
		},
	}
	step := &ApplyPvYmlServicesStep{}
	if _, err := step.Run(ctx); err == nil {
		t.Fatal("Run: want error when postgres not installed, got nil")
	}
}

func TestApplyPvYmlServices_ShouldRunFalseWithoutConfig(t *testing.T) {
	ctx := &automation.Context{}
	step := &ApplyPvYmlServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when ProjectConfig is nil")
	}
}

func TestApplyPvYmlServices_ShouldRunFalseWhenNoServicesDeclared(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &ApplyPvYmlServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when no services declared")
	}
}
