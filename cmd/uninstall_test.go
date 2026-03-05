package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func TestHasAuthTokens_WithTokens(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.json")
	data := `{"github-oauth":{"github.com":"ghp_abc123"}}`
	if err := os.WriteFile(path, []byte(data), 0644); err != nil {
		t.Fatal(err)
	}
	if !hasAuthTokens(path) {
		t.Error("expected hasAuthTokens to return true")
	}
}

func TestHasAuthTokens_Empty(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.json")
	if err := os.WriteFile(path, []byte("{}"), 0644); err != nil {
		t.Fatal(err)
	}
	if hasAuthTokens(path) {
		t.Error("expected hasAuthTokens to return false for empty object")
	}
}

func TestHasAuthTokens_Missing(t *testing.T) {
	if hasAuthTokens("/does/not/exist/auth.json") {
		t.Error("expected hasAuthTokens to return false for missing file")
	}
}

func TestCopyFile(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "src.json")
	dst := filepath.Join(dir, "dst.json")
	content := `{"key":"value"}`
	if err := os.WriteFile(src, []byte(content), 0644); err != nil {
		t.Fatal(err)
	}
	if err := copyFile(src, dst); err != nil {
		t.Fatalf("copyFile error = %v", err)
	}
	got, err := os.ReadFile(dst)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != content {
		t.Errorf("copied content = %q, want %q", string(got), content)
	}
}

func TestUninstall_RemovesPvDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Set up a minimal pv installation.
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	pvDir := config.PvDir()
	if _, err := os.Stat(pvDir); os.IsNotExist(err) {
		t.Fatal("expected ~/.pv to exist after EnsureDirs")
	}

	// Write a test registry with a project.
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "testapp", Path: filepath.Join(home, "projects", "testapp"), Type: "php"},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	// Write settings.
	settings := config.DefaultSettings()
	if err := settings.Save(); err != nil {
		t.Fatal(err)
	}

	// Verify RemoveAll works on the pv dir.
	if err := os.RemoveAll(pvDir); err != nil {
		t.Fatalf("RemoveAll error = %v", err)
	}
	if _, err := os.Stat(pvDir); !os.IsNotExist(err) {
		t.Error("expected ~/.pv to be removed")
	}
}

func TestUninstall_AuthBackup(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Write auth.json with tokens.
	authData := `{"github-oauth":{"github.com":"ghp_test"}}`
	authPath := filepath.Join(config.ComposerDir(), "auth.json")
	if err := os.WriteFile(authPath, []byte(authData), 0644); err != nil {
		t.Fatal(err)
	}

	// Simulate backup.
	backupPath := filepath.Join(home, "pv-auth-backup.json")
	if err := copyFile(authPath, backupPath); err != nil {
		t.Fatalf("copyFile error = %v", err)
	}

	// Verify backup exists and has correct content.
	got, err := os.ReadFile(backupPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != authData {
		t.Errorf("backup content = %q, want %q", string(got), authData)
	}

	// Now remove ~/.pv and verify backup survives.
	if err := os.RemoveAll(config.PvDir()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(backupPath); os.IsNotExist(err) {
		t.Error("backup should survive ~/.pv removal")
	}
}

func TestUninstall_RegistryReadBeforeDelete(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create projects and .pv-php files.
	projectPaths := []string{
		filepath.Join(home, "projects", "app-one"),
		filepath.Join(home, "projects", "app-two"),
	}
	var projects []registry.Project
	for _, p := range projectPaths {
		if err := os.MkdirAll(p, 0755); err != nil {
			t.Fatal(err)
		}
		projects = append(projects, registry.Project{Name: filepath.Base(p), Path: p, Type: "php"})
	}
	// Write .pv-php in first project only.
	if err := os.WriteFile(filepath.Join(projectPaths[0], ".pv-php"), []byte("8.3"), 0644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{Projects: projects}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	// Read registry (simulating Task 3).
	loaded, err := registry.Load()
	if err != nil {
		t.Fatal(err)
	}
	var paths []string
	for _, p := range loaded.List() {
		paths = append(paths, p.Path)
	}

	// Delete ~/.pv (simulating Task 7).
	if err := os.RemoveAll(config.PvDir()); err != nil {
		t.Fatal(err)
	}

	// Verify we can still find .pv-php files from saved paths (Task 8).
	var found []string
	for _, p := range paths {
		pvPhpPath := filepath.Join(p, ".pv-php")
		if _, err := os.Stat(pvPhpPath); err == nil {
			found = append(found, pvPhpPath)
		}
	}
	if len(found) != 1 {
		t.Errorf("expected 1 .pv-php file, found %d", len(found))
	}
}

func TestUninstall_SettingsLoadFallback(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// No settings file exists — LoadSettings should return defaults.
	settings, err := config.LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings error = %v", err)
	}
	if settings.TLD != "test" {
		t.Errorf("TLD = %q, want %q", settings.TLD, "test")
	}
}
