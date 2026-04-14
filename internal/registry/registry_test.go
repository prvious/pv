package registry

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// --- In-memory tests (no filesystem) ---

func TestAdd_ToEmpty(t *testing.T) {
	r := &Registry{}
	err := r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	if err != nil {
		t.Fatalf("Add() error = %v", err)
	}
	if len(r.Projects) != 1 {
		t.Fatalf("expected 1 project, got %d", len(r.Projects))
	}
	if r.Projects[0].Name != "foo" {
		t.Errorf("expected name %q, got %q", "foo", r.Projects[0].Name)
	}
}

func TestAdd_Duplicate(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	err := r.Add(Project{Name: "foo", Path: "/tmp/foo2"})
	if err == nil {
		t.Fatal("expected error for duplicate name, got nil")
	}
}

func TestAdd_MultipleUnique(t *testing.T) {
	r := &Registry{}
	for _, name := range []string{"a", "b", "c"} {
		if err := r.Add(Project{Name: name, Path: "/tmp/" + name}); err != nil {
			t.Fatalf("Add(%q) error = %v", name, err)
		}
	}
	if len(r.Projects) != 3 {
		t.Fatalf("expected 3 projects, got %d", len(r.Projects))
	}
}

func TestRemove_Existing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	if err := r.Remove("foo"); err != nil {
		t.Fatalf("Remove() error = %v", err)
	}
	if len(r.Projects) != 0 {
		t.Fatalf("expected 0 projects, got %d", len(r.Projects))
	}
}

func TestRemove_NonExistent(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	if err := r.Remove("bar"); err == nil {
		t.Fatal("expected error for non-existent, got nil")
	}
}

func TestRemove_FromEmpty(t *testing.T) {
	r := &Registry{}
	if err := r.Remove("foo"); err == nil {
		t.Fatal("expected error for empty registry, got nil")
	}
}

func TestRemove_First(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "a", Path: "/a"})
	_ = r.Add(Project{Name: "b", Path: "/b"})
	_ = r.Add(Project{Name: "c", Path: "/c"})

	if err := r.Remove("a"); err != nil {
		t.Fatalf("Remove() error = %v", err)
	}
	if len(r.Projects) != 2 {
		t.Fatalf("expected 2 projects, got %d", len(r.Projects))
	}
	if r.Projects[0].Name != "b" || r.Projects[1].Name != "c" {
		t.Errorf("unexpected order: %v", r.Projects)
	}
}

func TestRemove_Middle(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "a", Path: "/a"})
	_ = r.Add(Project{Name: "b", Path: "/b"})
	_ = r.Add(Project{Name: "c", Path: "/c"})

	if err := r.Remove("b"); err != nil {
		t.Fatalf("Remove() error = %v", err)
	}
	if len(r.Projects) != 2 {
		t.Fatalf("expected 2 projects, got %d", len(r.Projects))
	}
	if r.Projects[0].Name != "a" || r.Projects[1].Name != "c" {
		t.Errorf("unexpected order: %v", r.Projects)
	}
}

func TestRemove_Last(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "a", Path: "/a"})
	_ = r.Add(Project{Name: "b", Path: "/b"})
	_ = r.Add(Project{Name: "c", Path: "/c"})

	if err := r.Remove("c"); err != nil {
		t.Fatalf("Remove() error = %v", err)
	}
	if len(r.Projects) != 2 {
		t.Fatalf("expected 2 projects, got %d", len(r.Projects))
	}
	if r.Projects[0].Name != "a" || r.Projects[1].Name != "b" {
		t.Errorf("unexpected order: %v", r.Projects)
	}
}

func TestUpdateWith_Existing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo", Type: "php"})

	err := r.UpdateWith("foo", func(p *Project) {
		p.Path = "/tmp/foo2"
		p.Type = "laravel"
	})
	if err != nil {
		t.Fatalf("UpdateWith() error = %v", err)
	}
	if len(r.Projects) != 1 {
		t.Fatalf("expected 1 project, got %d", len(r.Projects))
	}
	if r.Projects[0].Path != "/tmp/foo2" {
		t.Errorf("expected path %q, got %q", "/tmp/foo2", r.Projects[0].Path)
	}
	if r.Projects[0].Type != "laravel" {
		t.Errorf("expected type %q, got %q", "laravel", r.Projects[0].Type)
	}
}

func TestUpdateWith_NonExistent(t *testing.T) {
	r := &Registry{}
	err := r.UpdateWith("foo", func(p *Project) {
		p.Path = "/tmp/foo"
	})
	if err == nil {
		t.Fatal("expected error for non-existent project, got nil")
	}
}

func TestUpdateWith_PreservesOrder(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "a", Path: "/tmp/a"})
	_ = r.Add(Project{Name: "b", Path: "/tmp/b"})
	_ = r.Add(Project{Name: "c", Path: "/tmp/c"})

	err := r.UpdateWith("b", func(p *Project) {
		p.Path = "/tmp/b2"
		p.Type = "laravel"
	})
	if err != nil {
		t.Fatalf("UpdateWith() error = %v", err)
	}
	if r.Projects[0].Name != "a" || r.Projects[1].Name != "b" || r.Projects[2].Name != "c" {
		t.Errorf("expected order [a b c], got [%s %s %s]", r.Projects[0].Name, r.Projects[1].Name, r.Projects[2].Name)
	}
	if r.Projects[1].Path != "/tmp/b2" {
		t.Errorf("expected updated path, got %q", r.Projects[1].Path)
	}
}

func TestUpdateWith_PreservesUntouchedFields(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{
		Name:      "foo",
		Path:      "/tmp/foo",
		Type:      "php",
		Services:  &ProjectServices{MySQL: "mysql:8.0"},
		Databases: []string{"foo_db"},
	})

	err := r.UpdateWith("foo", func(p *Project) {
		p.Path = "/tmp/foo2"
		p.Type = "laravel"
	})
	if err != nil {
		t.Fatalf("UpdateWith() error = %v", err)
	}
	if r.Projects[0].Services == nil || r.Projects[0].Services.MySQL != "mysql:8.0" {
		t.Errorf("expected Services preserved, got %v", r.Projects[0].Services)
	}
	if len(r.Projects[0].Databases) != 1 || r.Projects[0].Databases[0] != "foo_db" {
		t.Errorf("expected Databases preserved, got %v", r.Projects[0].Databases)
	}
}

func TestFind_Existing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	p := r.Find("foo")
	if p == nil {
		t.Fatal("Find() returned nil, want non-nil")
	}
	if p.Name != "foo" {
		t.Errorf("Find() name = %q, want %q", p.Name, "foo")
	}
}

func TestFind_Missing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	if p := r.Find("bar"); p != nil {
		t.Errorf("Find() = %v, want nil", p)
	}
}

func TestFind_Empty(t *testing.T) {
	r := &Registry{}
	if p := r.Find("foo"); p != nil {
		t.Errorf("Find() = %v, want nil", p)
	}
}

func TestFindByPath_Existing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	p := r.FindByPath("/tmp/foo")
	if p == nil {
		t.Fatal("FindByPath() returned nil, want non-nil")
	}
	if p.Path != "/tmp/foo" {
		t.Errorf("FindByPath() path = %q, want %q", p.Path, "/tmp/foo")
	}
}

func TestFindByPath_Missing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo"})
	if p := r.FindByPath("/tmp/bar"); p != nil {
		t.Errorf("FindByPath() = %v, want nil", p)
	}
}

func TestFindByPath_Empty(t *testing.T) {
	r := &Registry{}
	if p := r.FindByPath("/tmp/foo"); p != nil {
		t.Errorf("FindByPath() = %v, want nil", p)
	}
}

func TestList_Empty(t *testing.T) {
	r := &Registry{}
	projects := r.List()
	if len(projects) != 0 {
		t.Errorf("List() returned %d projects, want 0", len(projects))
	}
}

func TestList_Populated(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "a", Path: "/a"})
	_ = r.Add(Project{Name: "b", Path: "/b"})

	projects := r.List()
	if len(projects) != 2 {
		t.Fatalf("List() returned %d projects, want 2", len(projects))
	}
	if projects[0].Name != "a" || projects[1].Name != "b" {
		t.Errorf("List() order = %v, want a then b", projects)
	}
}

// --- Filesystem tests ---

func TestLoad_NoFile(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	reg, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if len(reg.Projects) != 0 {
		t.Errorf("expected empty registry, got %d projects", len(reg.Projects))
	}
}

func TestLoad_ValidJSON(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}
	data := `{"projects":[{"name":"myapp","path":"/srv/myapp","type":"laravel"}]}`
	if err := os.WriteFile(config.RegistryPath(), []byte(data), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	reg, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if len(reg.Projects) != 1 {
		t.Fatalf("expected 1 project, got %d", len(reg.Projects))
	}
	if reg.Projects[0].Name != "myapp" {
		t.Errorf("name = %q, want %q", reg.Projects[0].Name, "myapp")
	}
	if reg.Projects[0].Type != "laravel" {
		t.Errorf("type = %q, want %q", reg.Projects[0].Type, "laravel")
	}
}

func TestLoad_InvalidJSON(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}
	if err := os.WriteFile(config.RegistryPath(), []byte("not json"), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	_, err := Load()
	if err == nil {
		t.Fatal("expected error for invalid JSON, got nil")
	}
}

func TestSaveLoad_RoundTrip(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	reg := &Registry{}
	_ = reg.Add(Project{Name: "app1", Path: "/srv/app1", Type: "laravel"})
	_ = reg.Add(Project{Name: "app2", Path: "/srv/app2", Type: "static"})

	if err := reg.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	// Verify file exists
	if _, err := os.Stat(config.RegistryPath()); err != nil {
		t.Fatalf("registry file does not exist: %v", err)
	}

	loaded, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if len(loaded.Projects) != 2 {
		t.Fatalf("expected 2 projects, got %d", len(loaded.Projects))
	}
	for i, want := range reg.Projects {
		got := loaded.Projects[i]
		if got.Name != want.Name || got.Path != want.Path || got.Type != want.Type {
			t.Errorf("project[%d] = %+v, want %+v", i, got, want)
		}
	}
}

func TestSave_CreatesDirectories(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Data dir should not exist yet
	if _, err := os.Stat(filepath.Join(home, ".pv", "data")); !os.IsNotExist(err) {
		t.Fatal("data dir should not exist before Save()")
	}

	reg := &Registry{}
	if err := reg.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	if _, err := os.Stat(config.RegistryPath()); err != nil {
		t.Fatalf("registry file does not exist after Save(): %v", err)
	}
}

// --- Service CRUD tests ---

func TestAddService(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	err := r.AddService("mysql:8.0.32", &ServiceInstance{Image: "mysql:8.0.32", Port: 33032})
	if err != nil {
		t.Fatalf("AddService() error = %v", err)
	}
	if len(r.Services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(r.Services))
	}
}

func TestAddService_Duplicate(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("redis", &ServiceInstance{Image: "redis:latest", Port: 6379})
	err := r.AddService("redis", &ServiceInstance{Image: "redis:latest", Port: 6379})
	if err == nil {
		t.Fatal("expected error for duplicate service, got nil")
	}
}

func TestRemoveService(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("redis", &ServiceInstance{Image: "redis:latest", Port: 6379})
	err := r.RemoveService("redis")
	if err != nil {
		t.Fatalf("RemoveService() error = %v", err)
	}
	if len(r.Services) != 0 {
		t.Fatalf("expected 0 services, got %d", len(r.Services))
	}
}

func TestRemoveService_NotFound(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	err := r.RemoveService("mysql")
	if err == nil {
		t.Fatal("expected error for non-existent service")
	}
}

func TestFindService(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("redis", &ServiceInstance{Image: "redis:latest", Port: 6379})

	svc, err := r.FindService("redis")
	if err != nil {
		t.Fatalf("FindService(redis) error = %v", err)
	}
	if svc == nil {
		t.Fatal("FindService() returned nil")
	}
	if svc.Port != 6379 {
		t.Errorf("Port = %d, want 6379", svc.Port)
	}

	svc, err = r.FindService("mysql")
	if err != nil {
		t.Fatalf("FindService(mysql) unexpected error = %v", err)
	}
	if svc != nil {
		t.Error("FindService(mysql) should return nil")
	}
}

func TestFindService_FuzzyMatch(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("mysql:8.4", &ServiceInstance{Image: "mysql:8.4", Port: 33000})

	// Fuzzy match by name prefix.
	svc, err := r.FindService("mysql")
	if err != nil {
		t.Fatalf("FindService(mysql) error = %v", err)
	}
	if svc == nil {
		t.Fatal("FindService(mysql) returned nil, want fuzzy match for mysql:8.4")
	}
	if svc.Port != 33000 {
		t.Errorf("Port = %d, want 33000", svc.Port)
	}

	// Exact match still works.
	svc, err = r.FindService("mysql:8.4")
	if err != nil {
		t.Fatalf("FindService(mysql:8.4) error = %v", err)
	}
	if svc == nil {
		t.Fatal("FindService(mysql:8.4) returned nil")
	}

	// No match returns nil.
	svc, err = r.FindService("postgres")
	if err != nil {
		t.Fatalf("FindService(postgres) unexpected error = %v", err)
	}
	if svc != nil {
		t.Error("FindService(postgres) should return nil")
	}
}

func TestFindService_Ambiguous(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("mysql:8.0", &ServiceInstance{Image: "mysql:8.0", Port: 33000})
	_ = r.AddService("mysql:8.4", &ServiceInstance{Image: "mysql:8.4", Port: 33000})

	_, err := r.FindService("mysql")
	if err == nil {
		t.Fatal("FindService(mysql) should return ambiguity error when multiple versions exist")
	}
}

func TestResolveServiceKey_Exact(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("mysql:8.4", &ServiceInstance{Image: "mysql:8.4", Port: 33000})

	key, err := r.ResolveServiceKey("mysql:8.4")
	if err != nil {
		t.Fatalf("ResolveServiceKey() error = %v", err)
	}
	if key != "mysql:8.4" {
		t.Errorf("key = %q, want %q", key, "mysql:8.4")
	}
}

func TestResolveServiceKey_Prefix(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("postgres:18-alpine", &ServiceInstance{Image: "postgres:18-alpine", Port: 54018})

	key, err := r.ResolveServiceKey("postgres")
	if err != nil {
		t.Fatalf("ResolveServiceKey() error = %v", err)
	}
	if key != "postgres:18-alpine" {
		t.Errorf("key = %q, want %q", key, "postgres:18-alpine")
	}
}

func TestResolveServiceKey_NoMatch(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("mysql:8.4", &ServiceInstance{Image: "mysql:8.4", Port: 33000})

	key, err := r.ResolveServiceKey("redis")
	if err != nil {
		t.Fatalf("ResolveServiceKey() error = %v", err)
	}
	// Returns original key when not found.
	if key != "redis" {
		t.Errorf("key = %q, want %q", key, "redis")
	}
}

func TestResolveServiceKey_Ambiguous(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("mysql:8.0", &ServiceInstance{Image: "mysql:8.0", Port: 33000})
	_ = r.AddService("mysql:8.4", &ServiceInstance{Image: "mysql:8.4", Port: 33000})

	_, err := r.ResolveServiceKey("mysql")
	if err == nil {
		t.Fatal("expected error for ambiguous match, got nil")
	}
}

func TestProjectsUsingService(t *testing.T) {
	r := &Registry{
		Services: make(map[string]*ServiceInstance),
		Projects: []Project{
			{Name: "app1", Path: "/a", Services: &ProjectServices{MySQL: "8.0.32", Redis: true}},
			{Name: "app2", Path: "/b", Services: &ProjectServices{MySQL: "8.0.32"}},
			{Name: "app3", Path: "/c"},
		},
	}

	mysqlProjects := r.ProjectsUsingService("mysql")
	if len(mysqlProjects) != 2 {
		t.Errorf("expected 2 mysql projects, got %d", len(mysqlProjects))
	}

	redisProjects := r.ProjectsUsingService("redis")
	if len(redisProjects) != 1 {
		t.Errorf("expected 1 redis project, got %d", len(redisProjects))
	}

	pgProjects := r.ProjectsUsingService("postgres")
	if len(pgProjects) != 0 {
		t.Errorf("expected 0 postgres projects, got %d", len(pgProjects))
	}
}

func TestUnbindService(t *testing.T) {
	r := &Registry{
		Services: make(map[string]*ServiceInstance),
		Projects: []Project{
			{Name: "app1", Path: "/a", Services: &ProjectServices{MySQL: "8.0.32"}},
			{Name: "app2", Path: "/b", Services: &ProjectServices{MySQL: "8.0.32"}},
		},
	}
	r.UnbindService("mysql")
	for _, p := range r.Projects {
		if p.Services != nil && p.Services.MySQL != "" {
			t.Errorf("project %s still has MySQL binding", p.Name)
		}
	}
}

func TestLoad_BackwardCompat_NoServicesField(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	// Old-format JSON without services field.
	data := `{"projects":[{"name":"myapp","path":"/srv/myapp","type":"laravel"}]}`
	if err := os.WriteFile(config.RegistryPath(), []byte(data), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	reg, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if reg.Services == nil {
		t.Fatal("Services map should be initialized")
	}
	if len(reg.Services) != 0 {
		t.Errorf("expected 0 services, got %d", len(reg.Services))
	}
	if len(reg.Projects) != 1 {
		t.Errorf("expected 1 project, got %d", len(reg.Projects))
	}
}

func TestLoad_BackwardCompat_ContainerID(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	// Old-format JSON with container_id field (now removed).
	data := `{"services":{"mysql:8.0":{"image":"mysql:8.0","port":33000,"container_id":"abc123"}},"projects":[]}`
	if err := os.WriteFile(config.RegistryPath(), []byte(data), 0644); err != nil {
		t.Fatalf("WriteFile error = %v", err)
	}

	reg, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v (old container_id field should be silently ignored)", err)
	}
	if len(reg.Services) != 1 {
		t.Fatalf("expected 1 service, got %d", len(reg.Services))
	}
	if reg.Services["mysql:8.0"].Port != 33000 {
		t.Errorf("port = %d, want 33000", reg.Services["mysql:8.0"].Port)
	}
}

func TestServiceSaveLoad_RoundTrip(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	reg := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = reg.Add(Project{Name: "app1", Path: "/srv/app1", Type: "laravel"})
	_ = reg.AddService("mysql:8.0.32", &ServiceInstance{
		Image: "mysql:8.0.32",
		Port:  33032,
	})
	_ = reg.AddService("redis", &ServiceInstance{
		Image: "redis:latest",
		Port:  6379,
	})

	if err := reg.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	loaded, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}

	if len(loaded.Services) != 2 {
		t.Fatalf("expected 2 services, got %d", len(loaded.Services))
	}
	if loaded.Services["mysql:8.0.32"].Port != 33032 {
		t.Errorf("mysql port = %d, want 33032", loaded.Services["mysql:8.0.32"].Port)
	}
	if loaded.Services["redis"].Port != 6379 {
		t.Errorf("redis port = %d, want 6379", loaded.Services["redis"].Port)
	}
}

func TestServiceInstance_JSON_WithKindEnabled(t *testing.T) {
	enabled := true
	si := ServiceInstance{
		Image:       "",
		Port:        9000,
		ConsolePort: 9001,
		Kind:        "binary",
		Enabled:     &enabled,
	}
	data, err := json.Marshal(si)
	if err != nil {
		t.Fatal(err)
	}
	var back ServiceInstance
	if err := json.Unmarshal(data, &back); err != nil {
		t.Fatal(err)
	}
	if back.Kind != "binary" {
		t.Errorf("Kind round-trip: got %q", back.Kind)
	}
	if back.Enabled == nil || *back.Enabled != true {
		t.Errorf("Enabled round-trip: got %v", back.Enabled)
	}
}

func TestServiceInstance_JSON_OldFormat_DefaultsToDocker(t *testing.T) {
	// Entries written by earlier pv versions do not include Kind or Enabled.
	blob := []byte(`{"image":"redis:7","port":6379}`)
	var si ServiceInstance
	if err := json.Unmarshal(blob, &si); err != nil {
		t.Fatal(err)
	}
	if si.Kind != "" {
		t.Errorf("old entry should deserialize with empty Kind; got %q", si.Kind)
	}
	if si.Enabled != nil {
		t.Errorf("old entry should deserialize with nil Enabled; got %v", si.Enabled)
	}
}

func TestServiceInstance_JSON_EmptyFields_Omitted(t *testing.T) {
	si := ServiceInstance{Image: "redis:7", Port: 6379}
	data, _ := json.Marshal(si)
	s := string(data)
	if strings.Contains(s, "kind") {
		t.Errorf("expected kind to be omitted when empty; got %s", s)
	}
	if strings.Contains(s, "enabled") {
		t.Errorf("expected enabled to be omitted when nil; got %s", s)
	}
}
