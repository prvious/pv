package steps

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// stageMysqlBinary writes a stub mysqld at ~/.pv/mysql/<version>/bin/mysqld
// so mysql.InstalledVersions() returns it.
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

func TestDetectServices_BindsMysqlWhenExplicit(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageMysqlBinary(t, "8.4")

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".env"),
		[]byte("DB_CONNECTION=mysql\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectType: "laravel",
		Registry:    reg,
	}
	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	got := ""
	if reg.Projects[0].Services != nil {
		got = reg.Projects[0].Services.MySQL
	}
	if got != "8.4" {
		t.Errorf("MySQL binding = %q, want %q", got, "8.4")
	}
}

func TestDetectServices_DoesNotBindMysqlWhenUnset(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageMysqlBinary(t, "8.4")

	projDir := t.TempDir()
	// .env exists but has no DB_CONNECTION at all.
	if err := os.WriteFile(filepath.Join(projDir, ".env"),
		[]byte("APP_NAME=demo\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectType: "laravel",
		Registry:    reg,
	}
	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services != nil && reg.Projects[0].Services.MySQL != "" {
		t.Errorf("MySQL binding = %q, want empty (DB_CONNECTION unset must not auto-bind)",
			reg.Projects[0].Services.MySQL)
	}
}

func TestDetectServices_DoesNotBindMysqlWhenOtherDriver(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageMysqlBinary(t, "8.4")

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".env"),
		[]byte("DB_CONNECTION=sqlite\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectType: "laravel",
		Registry:    reg,
	}
	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services != nil && reg.Projects[0].Services.MySQL != "" {
		t.Errorf("MySQL binding = %q, want empty (DB_CONNECTION=sqlite must not bind mysql)",
			reg.Projects[0].Services.MySQL)
	}
}

func TestDetectServices_AutoBindsRedisWhenInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Pre-stage redis as installed.
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}

	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	// .env has NO REDIS_HOST — auto-bind must trigger anyway (mirrors mailpit/rustfs).
	if err := os.WriteFile(envPath, []byte("APP_NAME=test\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "test", Path: dir, Type: "laravel"},
		},
	}

	ctx := &automation.Context{
		ProjectName: "test",
		ProjectPath: dir,
		ProjectType: "laravel",
		Registry:    reg,
	}

	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if reg.Projects[0].Services == nil || !reg.Projects[0].Services.Redis {
		t.Errorf("project should have Redis=true after detect when redis is installed")
	}
}

func TestDetectServices_PrefersHighestMysqlVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageMysqlBinary(t, "8.0")
	stageMysqlBinary(t, "8.4")
	stageMysqlBinary(t, "9.7")

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".env"),
		[]byte("DB_CONNECTION=mysql\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{Name: "p", Path: projDir, Type: "laravel"}},
	}
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectType: "laravel",
		Registry:    reg,
	}
	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	got := ""
	if reg.Projects[0].Services != nil {
		got = reg.Projects[0].Services.MySQL
	}
	if got != "9.7" {
		t.Errorf("MySQL binding = %q, want %q (highest)", got, "9.7")
	}
}
