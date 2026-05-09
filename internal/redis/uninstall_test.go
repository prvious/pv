package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func setupInstalledRedis(t *testing.T) {
	t.Helper()
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDir(), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(config.RedisDataDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDataDir(), "dump.rdb"), []byte("fake"), 0o644); err != nil {
		t.Fatal(err)
	}
	_ = SetWanted(WantedRunning)
}

func TestUninstall_NoForce_KeepsDatadir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t)

	if err := Uninstall(false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(config.RedisDir()); !os.IsNotExist(err) {
		t.Errorf("RedisDir should be removed: err=%v", err)
	}
	if _, err := os.Stat(config.RedisDataDir()); err != nil {
		t.Errorf("RedisDataDir should remain: %v", err)
	}
	st, _ := LoadState()
	if st.Wanted != "" {
		t.Errorf("state should be cleared, got Wanted=%q", st.Wanted)
	}
}

func TestUninstall_Force_RemovesDatadir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t)

	if err := Uninstall(true); err != nil {
		t.Fatalf("Uninstall(force): %v", err)
	}
	if _, err := os.Stat(config.RedisDataDir()); !os.IsNotExist(err) {
		t.Errorf("RedisDataDir should be removed with --force: err=%v", err)
	}
}

func TestUninstall_UnbindsProjects(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t)

	// Pre-load a project bound to redis.
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "foo", Path: "/tmp/foo", Type: "laravel", Services: &registry.ProjectServices{Redis: true}},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	if err := Uninstall(false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}

	r2, _ := registry.Load()
	if r2.Projects[0].Services != nil && r2.Projects[0].Services.Redis {
		t.Errorf("project should have Redis=false after uninstall")
	}
}
