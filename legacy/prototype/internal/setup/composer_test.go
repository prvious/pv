package setup

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestMigrateComposerConfig_CopiesAuthJson(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create ~/.composer/auth.json.
	oldDir := filepath.Join(home, ".composer")
	os.MkdirAll(oldDir, 0755)
	authContent := `{"github-oauth":{"github.com":"test-token"}}`
	os.WriteFile(filepath.Join(oldDir, "auth.json"), []byte(authContent), 0600)

	MigrateComposerConfig()

	dst := filepath.Join(config.ComposerDir(), "auth.json")
	data, err := os.ReadFile(dst)
	if err != nil {
		t.Fatalf("auth.json not migrated: %v", err)
	}
	if string(data) != authContent {
		t.Errorf("auth.json content = %q, want %q", string(data), authContent)
	}
}

func TestMigrateComposerConfig_CopiesConfigJson(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	oldDir := filepath.Join(home, ".composer")
	os.MkdirAll(oldDir, 0755)
	configContent := `{"repositories":[{"type":"composer","url":"https://repo.example.com"}]}`
	os.WriteFile(filepath.Join(oldDir, "config.json"), []byte(configContent), 0600)

	MigrateComposerConfig()

	dst := filepath.Join(config.ComposerDir(), "config.json")
	data, err := os.ReadFile(dst)
	if err != nil {
		t.Fatalf("config.json not migrated: %v", err)
	}
	if string(data) != configContent {
		t.Errorf("config.json content = %q, want %q", string(data), configContent)
	}
}

func TestMigrateComposerConfig_SkipsIfNoOldDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Should not panic or error when ~/.composer doesn't exist.
	MigrateComposerConfig()

	// Verify nothing was created.
	entries, _ := os.ReadDir(config.ComposerDir())
	// Only "cache" subdir should exist (created by EnsureDirs).
	for _, e := range entries {
		if e.Name() != "cache" {
			t.Errorf("unexpected file in composer dir: %s", e.Name())
		}
	}
}

func TestMigrateComposerConfig_DoesNotOverwriteExisting(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Create existing auth.json in pv.
	existingContent := `{"existing":"true"}`
	os.WriteFile(filepath.Join(config.ComposerDir(), "auth.json"), []byte(existingContent), 0600)

	// Create old auth.json.
	oldDir := filepath.Join(home, ".composer")
	os.MkdirAll(oldDir, 0755)
	os.WriteFile(filepath.Join(oldDir, "auth.json"), []byte(`{"old":"true"}`), 0600)

	MigrateComposerConfig()

	// Should keep existing, not overwrite.
	data, _ := os.ReadFile(filepath.Join(config.ComposerDir(), "auth.json"))
	if string(data) != existingContent {
		t.Errorf("existing auth.json was overwritten: got %q", string(data))
	}
}

func TestMigrateComposerConfig_FilePermissions(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	oldDir := filepath.Join(home, ".composer")
	os.MkdirAll(oldDir, 0755)
	os.WriteFile(filepath.Join(oldDir, "auth.json"), []byte(`{}`), 0600)

	MigrateComposerConfig()

	info, err := os.Stat(filepath.Join(config.ComposerDir(), "auth.json"))
	if err != nil {
		t.Fatal(err)
	}
	// Should be 0600 (owner read/write only) since it contains credentials.
	if info.Mode().Perm() != 0600 {
		t.Errorf("migrated auth.json permissions = %o, want 0600", info.Mode().Perm())
	}
}
