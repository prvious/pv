package steps

import (
	"os"
	"path/filepath"
	"strings"
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

// stageMysqlBinary writes a stub mysqld at ~/.pv/mysql/<version>/bin/mysqld
// so mysql.IsInstalled(version) returns true.
func stageMysqlBinary(t *testing.T, version string) {
	t.Helper()
	bin := config.MysqlBinDir(version)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", bin, err)
	}
	if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage mysqld: %v", err)
	}
}

func stageRedisBinary(t *testing.T, version string) {
	t.Helper()
	versionDir := config.RedisVersionDir(version)
	if err := os.MkdirAll(versionDir, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", versionDir, err)
	}
	if err := os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage redis-server: %v", err)
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
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error when postgres not installed, got nil")
	}
	if !strings.Contains(err.Error(), "pv postgres:install 18") {
		t.Errorf("err = %v; want it to include `pv postgres:install 18`", err)
	}
}

func TestApplyPvYmlServices_BindsMysqlFromConfig(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageMysqlBinary(t, "8.4")

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
			Mysql: &config.ServiceConfig{Version: "8.4"},
		},
	}
	step := &ApplyPvYmlServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services == nil || reg.Projects[0].Services.MySQL != "8.4" {
		t.Errorf("MySQL binding = %+v, want version=8.4", reg.Projects[0].Services)
	}
}

func TestApplyPvYmlServices_ErrorsWhenMysqlNotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// No stub — mysql.IsInstalled("8.4") returns false.

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
			Mysql: &config.ServiceConfig{Version: "8.4"},
		},
	}
	step := &ApplyPvYmlServicesStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error when mysql not installed, got nil")
	}
	if !strings.Contains(err.Error(), "pv mysql:install 8.4") {
		t.Errorf("err = %v; want it to include `pv mysql:install 8.4`", err)
	}
}

func TestApplyPvYmlServices_BindsRedisDefaultVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageRedisBinary(t, config.RedisDefaultVersion())

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
			Redis: &config.ServiceConfig{},
		},
	}
	step := &ApplyPvYmlServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services == nil || reg.Projects[0].Services.Redis != config.RedisDefaultVersion() {
		t.Errorf("Redis binding = %+v, want version=%s", reg.Projects[0].Services, config.RedisDefaultVersion())
	}
}

func TestApplyPvYmlServices_RejectsUnsupportedRedisVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageRedisBinary(t, config.RedisDefaultVersion())

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
			Redis: &config.ServiceConfig{Version: "7.4"},
		},
	}
	step := &ApplyPvYmlServicesStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error for unsupported redis version")
	}
	if !strings.Contains(err.Error(), "unsupported redis version") {
		t.Errorf("err = %v; want unsupported redis version", err)
	}
}

func TestApplyPvYmlServices_ErrorsWhenPostgresVersionEmpty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

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
			Postgresql: &config.ServiceConfig{}, // no Version
		},
	}
	step := &ApplyPvYmlServicesStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error when postgres version is empty, got nil")
	}
	if !strings.Contains(err.Error(), "version is required") {
		t.Errorf("err = %v; want it to include `version is required`", err)
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
