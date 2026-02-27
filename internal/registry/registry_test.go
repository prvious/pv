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
