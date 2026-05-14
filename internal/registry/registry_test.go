package registry

import (
	"os"
	"path/filepath"
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
		Services:  &ProjectServices{MySQL: "8.4"},
		Databases: []string{"foo_db"},
	})

	err := r.UpdateWith("foo", func(p *Project) {
		p.Path = "/tmp/foo2"
		p.Type = "laravel"
	})
	if err != nil {
		t.Fatalf("UpdateWith() error = %v", err)
	}
	if r.Projects[0].Services == nil || r.Projects[0].Services.MySQL != "8.4" {
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

func TestFindService_RequiresExactVersionedKey(t *testing.T) {
	r := &Registry{Services: make(map[string]*ServiceInstance)}
	_ = r.AddService("mysql:8.4", &ServiceInstance{Image: "mysql:8.4", Port: 33000})

	svc, err := r.FindService("mysql")
	if err != nil {
		t.Fatalf("FindService(mysql) unexpected error = %v", err)
	}
	if svc != nil {
		t.Fatal("FindService(mysql) should require exact versioned key")
	}

	svc, err = r.FindService("mysql:8.4")
	if err != nil {
		t.Fatalf("FindService(mysql:8.4) error = %v", err)
	}
	if svc == nil {
		t.Fatal("FindService(mysql:8.4) returned nil")
	}
}

func TestProjectsUsingService(t *testing.T) {
	r := &Registry{
		Services: make(map[string]*ServiceInstance),
		Projects: []Project{
			{Name: "app1", Path: "/a", Services: &ProjectServices{MySQL: "8.4", Redis: "8.6", Mail: "latest", S3: "latest"}},
			{Name: "app2", Path: "/b", Services: &ProjectServices{MySQL: "8.4"}},
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

	mailProjects := r.ProjectsUsingService("mail")
	if len(mailProjects) != 1 {
		t.Errorf("expected 1 mail project, got %d", len(mailProjects))
	}

	s3Projects := r.ProjectsUsingService("s3")
	if len(s3Projects) != 1 {
		t.Errorf("expected 1 s3 project, got %d", len(s3Projects))
	}
}

func TestUnbindService(t *testing.T) {
	r := &Registry{
		Services: make(map[string]*ServiceInstance),
		Projects: []Project{
			{Name: "app1", Path: "/a", Services: &ProjectServices{MySQL: "8.4", Mail: "latest", S3: "latest"}},
			{Name: "app2", Path: "/b", Services: &ProjectServices{MySQL: "8.4"}},
		},
	}
	r.UnbindService("mysql")
	for _, p := range r.Projects {
		if p.Services != nil && p.Services.MySQL != "" {
			t.Errorf("project %s still has MySQL binding", p.Name)
		}
	}
	r.UnbindService("mail")
	for _, p := range r.Projects {
		if p.Services != nil && p.Services.Mail != "" {
			t.Errorf("project %s still has Mail binding", p.Name)
		}
	}
	r.UnbindService("s3")
	for _, p := range r.Projects {
		if p.Services != nil && p.Services.S3 != "" {
			t.Errorf("project %s still has S3 binding", p.Name)
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

func TestUnbindPostgresMajor(t *testing.T) {
	r := &Registry{
		Services: map[string]*ServiceInstance{},
		Projects: []Project{
			{Name: "a", Services: &ProjectServices{Postgres: "17"}},
			{Name: "b", Services: &ProjectServices{Postgres: "18"}},
			{Name: "c", Services: &ProjectServices{Postgres: "17"}},
			{Name: "d", Services: nil},
		},
	}
	r.UnbindPostgresMajor("17")
	cases := map[string]string{"a": "", "b": "18", "c": ""}
	for name, want := range cases {
		got := ""
		for _, p := range r.Projects {
			if p.Name == name && p.Services != nil {
				got = p.Services.Postgres
			}
		}
		if got != want {
			t.Errorf("project %s.Postgres = %q, want %q", name, got, want)
		}
	}
}

func TestUnbindMysqlVersion(t *testing.T) {
	r := &Registry{
		Services: map[string]*ServiceInstance{},
		Projects: []Project{
			{Name: "a", Services: &ProjectServices{MySQL: "8.4"}},
			{Name: "b", Services: &ProjectServices{MySQL: "9.7"}},
			{Name: "c", Services: &ProjectServices{MySQL: "8.4"}},
			{Name: "d", Services: nil},
		},
	}
	r.UnbindMysqlVersion("8.4")
	cases := map[string]string{"a": "", "b": "9.7", "c": ""}
	for name, want := range cases {
		got := ""
		for _, p := range r.Projects {
			if p.Name == name && p.Services != nil {
				got = p.Services.MySQL
			}
		}
		if got != want {
			t.Errorf("project %s.MySQL = %q, want %q", name, got, want)
		}
	}
	for _, p := range r.Projects {
		if p.Name == "d" && p.Services != nil {
			t.Errorf("project d.Services should remain nil, got %+v", p.Services)
		}
	}
}

func TestUnbindRedisVersion(t *testing.T) {
	r := &Registry{
		Services: map[string]*ServiceInstance{},
		Projects: []Project{
			{Name: "a", Services: &ProjectServices{Redis: "8.6"}},
			{Name: "b", Services: &ProjectServices{Redis: "7.4"}},
			{Name: "c", Services: &ProjectServices{Redis: "8.6"}},
			{Name: "d", Services: nil},
		},
	}
	r.UnbindRedisVersion("8.6")
	cases := []struct {
		name string
		want string
	}{
		{"a", ""},
		{"b", "7.4"},
		{"c", ""},
	}
	for _, tc := range cases {
		for _, p := range r.Projects {
			if p.Name == tc.name {
				if p.Services == nil {
					t.Errorf("%s: Services is nil", tc.name)
				} else if p.Services.Redis != tc.want {
					t.Errorf("%s: Redis = %q, want %q", tc.name, p.Services.Redis, tc.want)
				}
			}
		}
	}
	for _, p := range r.Projects {
		if p.Name == "d" && p.Services != nil {
			t.Error("project d: Services should be nil")
		}
	}
}

func TestUnbindMailVersion(t *testing.T) {
	r := &Registry{
		Projects: []Project{
			{Name: "a", Services: &ProjectServices{Mail: "latest"}},
			{Name: "b", Services: &ProjectServices{Mail: "future"}},
			{Name: "c", Services: &ProjectServices{Mail: "latest"}},
			{Name: "d"},
		},
	}

	r.UnbindMailVersion("latest")

	cases := map[string]string{"a": "", "b": "future", "c": ""}
	for _, p := range r.Projects {
		if p.Services == nil {
			continue
		}
		if got := p.Services.Mail; got != cases[p.Name] {
			t.Errorf("%s: Mail = %q, want %q", p.Name, got, cases[p.Name])
		}
	}
}

func TestUnbindS3Version(t *testing.T) {
	r := &Registry{
		Projects: []Project{
			{Name: "a", Services: &ProjectServices{S3: "latest"}},
			{Name: "b", Services: &ProjectServices{S3: "future"}},
			{Name: "c", Services: &ProjectServices{S3: "latest"}},
			{Name: "d"},
		},
	}

	r.UnbindS3Version("latest")

	cases := map[string]string{"a": "", "b": "future", "c": ""}
	for _, p := range r.Projects {
		if p.Services == nil {
			continue
		}
		if got := p.Services.S3; got != cases[p.Name] {
			t.Errorf("%s: S3 = %q, want %q", p.Name, got, cases[p.Name])
		}
	}
}
