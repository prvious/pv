package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func setupInstalledRedis(t *testing.T, version string) {
	t.Helper()
	if err := os.MkdirAll(config.RedisVersionDir(version), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisVersionDir(version), "redis-server"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(config.RedisDataDirV(version), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(config.RedisDataDirV(version), "dump.rdb"), []byte("fake"), 0o644); err != nil {
		t.Fatal(err)
	}
	_ = SetWanted(version, WantedRunning)
}

func TestUninstall_NoForce_KeepsDatadir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t, "8.6")

	if err := Uninstall("8.6", false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(config.RedisVersionDir("8.6")); !os.IsNotExist(err) {
		t.Errorf("RedisVersionDir should be removed: err=%v", err)
	}
	if _, err := os.Stat(config.RedisDataDirV("8.6")); err != nil {
		t.Errorf("RedisDataDirV should remain: %v", err)
	}
	st, _ := LoadState()
	if len(st.Versions) != 0 {
		t.Errorf("state should be cleared, got %d versions", len(st.Versions))
	}
}

func TestUninstall_Force_RemovesDatadir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t, "8.6")

	if err := Uninstall("8.6", true); err != nil {
		t.Fatalf("Uninstall(force): %v", err)
	}
	if _, err := os.Stat(config.RedisDataDirV("8.6")); !os.IsNotExist(err) {
		t.Errorf("RedisDataDirV should be removed with --force: err=%v", err)
	}
}

func TestUninstall_UnbindsProjects(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t, "8.6")

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "foo", Path: "/tmp/foo", Type: "laravel", Services: &registry.ProjectServices{Redis: "8.6"}},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	if err := Uninstall("8.6", false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}

	r2, _ := registry.Load()
	if r2.Projects[0].Services != nil && r2.Projects[0].Services.Redis != "" {
		t.Errorf("project should have Redis unbound after uninstall")
	}
}

func TestUninstall_PropagatesRegistryLoadError(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupInstalledRedis(t, "8.6")

	if err := os.MkdirAll(filepath.Dir(config.RegistryPath()), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(config.RegistryPath(), 0o755); err != nil {
		t.Fatal(err)
	}

	if err := Uninstall("8.6", false); err == nil {
		t.Fatal("Uninstall: want registry load error")
	}
	if _, err := os.Stat(config.RedisVersionDir("8.6")); err != nil {
		t.Fatalf("redis binary dir should remain after registry error: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if st.Versions["8.6"].Wanted != WantedStopped {
		t.Fatalf("redis state should remain stopped after registry error, got %#v", st.Versions["8.6"])
	}
}
