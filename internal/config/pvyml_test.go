package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadProjectConfig_ValidPHP(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestLoadProjectConfig_UnquotedValue(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: 8.4\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestLoadProjectConfig_SingleQuoted(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: '8.4'\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestLoadProjectConfig_WithComment(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.3\" # pinned for legacy\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.3")
	}
}

func TestLoadProjectConfig_EmptyPHP(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("# empty config\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "" {
		t.Errorf("PHP = %q, want empty", cfg.PHP)
	}
}

func TestLoadProjectConfig_InvalidYAML(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: [invalid\n"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := LoadProjectConfig(path)
	if err == nil {
		t.Error("expected error for invalid YAML")
	}
}

func TestLoadProjectConfig_FileNotFound(t *testing.T) {
	_, err := LoadProjectConfig("/nonexistent/pv.yml")
	if err == nil {
		t.Error("expected error for missing file")
	}
}

func TestLoadProjectConfig_ExtraWhitespace(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php:   \"8.4\"  \n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestFindProjectConfig_InCurrentDir(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	got := FindProjectConfig(dir)
	if got != path {
		t.Errorf("FindProjectConfig() = %q, want %q", got, path)
	}
}

func TestFindProjectConfig_InParentDir(t *testing.T) {
	parent := t.TempDir()
	child := filepath.Join(parent, "sub", "deep")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}

	pvPath := filepath.Join(parent, ProjectConfigFilename)
	if err := os.WriteFile(pvPath, []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	got := FindProjectConfig(child)
	if got != pvPath {
		t.Errorf("FindProjectConfig() = %q, want %q", got, pvPath)
	}
}

func TestFindProjectConfig_ClosestWins(t *testing.T) {
	parent := t.TempDir()
	child := filepath.Join(parent, "sub")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}

	if err := os.WriteFile(filepath.Join(parent, ProjectConfigFilename), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	childPath := filepath.Join(child, ProjectConfigFilename)
	if err := os.WriteFile(childPath, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	got := FindProjectConfig(child)
	if got != childPath {
		t.Errorf("FindProjectConfig() = %q, want %q (closest should win)", got, childPath)
	}
}

func TestFindProjectConfig_NotFound(t *testing.T) {
	dir := t.TempDir()

	got := FindProjectConfig(dir)
	if got != "" {
		t.Errorf("FindProjectConfig() = %q, want empty (no pv.yml)", got)
	}
}

func TestFindAndLoadProjectConfig_Found(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, ProjectConfigFilename), []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := FindAndLoadProjectConfig(dir)
	if err != nil {
		t.Fatalf("FindAndLoadProjectConfig() error = %v", err)
	}
	if cfg == nil {
		t.Fatal("FindAndLoadProjectConfig() returned nil config")
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestFindAndLoadProjectConfig_NotFound(t *testing.T) {
	dir := t.TempDir()

	cfg, err := FindAndLoadProjectConfig(dir)
	if err != nil {
		t.Fatalf("FindAndLoadProjectConfig() error = %v", err)
	}
	if cfg != nil {
		t.Errorf("FindAndLoadProjectConfig() = %v, want nil when no pv.yml", cfg)
	}
}

func TestFindAndLoadProjectConfig_WalksUp(t *testing.T) {
	parent := t.TempDir()
	child := filepath.Join(parent, "a", "b", "c")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(parent, ProjectConfigFilename), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := FindAndLoadProjectConfig(child)
	if err != nil {
		t.Fatalf("FindAndLoadProjectConfig() error = %v", err)
	}
	if cfg == nil {
		t.Fatal("FindAndLoadProjectConfig() returned nil config")
	}
	if cfg.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.3")
	}
}

func TestFindAndLoadProjectConfig_InvalidYAML(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, ProjectConfigFilename), []byte("php: [broken\n"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := FindAndLoadProjectConfig(dir)
	if err == nil {
		t.Error("expected error for invalid YAML")
	}
}
