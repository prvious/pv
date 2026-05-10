package redis

import (
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
)

func init() {
	EnvWriter = func(projectPath, projectName string, bound *registry.ProjectServices) error {
		envPath := filepath.Join(projectPath, ".env")
		if _, err := os.Stat(envPath); os.IsNotExist(err) {
			return nil
		}
		return projectenv.MergeDotEnv(envPath, "", map[string]string{
			"REDIS_HOST":     "127.0.0.1",
			"REDIS_PORT":     strconv.Itoa(PortFor()),
			"REDIS_PASSWORD": "null",
		})
	}
}

func writeProjectEnv(t *testing.T, dir, content string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestBindLinkedProjects_LaravelOnly(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	a := filepath.Join(t.TempDir(), "a")
	b := filepath.Join(t.TempDir(), "b")
	c := filepath.Join(t.TempDir(), "c")
	writeProjectEnv(t, a, "APP_NAME=a\n")
	writeProjectEnv(t, b, "APP_NAME=b\n")
	writeProjectEnv(t, c, "APP_NAME=c\n")

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "a", Path: a, Type: "laravel"},
			{Name: "b", Path: b, Type: "laravel-octane"},
			{Name: "c", Path: c, Type: "static"}, // not Laravel — should not bind
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	if err := BindLinkedProjects(); err != nil {
		t.Fatalf("BindLinkedProjects: %v", err)
	}

	r2, _ := registry.Load()
	if r2.Projects[0].Services == nil || !r2.Projects[0].Services.Redis {
		t.Errorf("project a (laravel) should have Redis=true")
	}
	if r2.Projects[1].Services == nil || !r2.Projects[1].Services.Redis {
		t.Errorf("project b (laravel-octane) should have Redis=true")
	}
	if r2.Projects[2].Services != nil && r2.Projects[2].Services.Redis {
		t.Errorf("project c (static) must NOT have Redis bound")
	}

	// .env files for laravel projects should have REDIS_HOST set.
	for _, p := range []string{a, b} {
		data, err := os.ReadFile(filepath.Join(p, ".env"))
		if err != nil {
			t.Fatal(err)
		}
		if !contains(string(data), "REDIS_HOST=127.0.0.1") {
			t.Errorf("project at %s missing REDIS_HOST=127.0.0.1, .env=%s", p, string(data))
		}
	}
}

func contains(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
