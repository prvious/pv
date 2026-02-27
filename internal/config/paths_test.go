package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPvDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PvDir()
	if got != filepath.Join(home, ".pv") {
		t.Errorf("PvDir() = %q, want %q", got, filepath.Join(home, ".pv"))
	}
}

func TestConfigDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ConfigDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "config")) {
		t.Errorf("ConfigDir() = %q, want suffix .pv/config", got)
	}
}

func TestSitesDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := SitesDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "config", "sites")) {
		t.Errorf("SitesDir() = %q, want suffix .pv/config/sites", got)
	}
}

func TestLogsDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := LogsDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "logs")) {
		t.Errorf("LogsDir() = %q, want suffix .pv/logs", got)
	}
}

func TestDataDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := DataDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "data")) {
		t.Errorf("DataDir() = %q, want suffix .pv/data", got)
	}
}

func TestBinDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := BinDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "bin")) {
		t.Errorf("BinDir() = %q, want suffix .pv/bin", got)
	}
}

func TestRegistryPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := RegistryPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "data", "registry.json")) {
		t.Errorf("RegistryPath() = %q, want suffix .pv/data/registry.json", got)
	}
}

func TestEnsureDirs(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	dirs := []string{
		ConfigDir(),
		SitesDir(),
		LogsDir(),
		DataDir(),
		BinDir(),
	}
	for _, dir := range dirs {
		info, err := os.Stat(dir)
		if err != nil {
			t.Errorf("directory %q does not exist after EnsureDirs()", dir)
			continue
		}
		if !info.IsDir() {
			t.Errorf("%q is not a directory", dir)
		}
	}
}

func TestEnsureDirs_Idempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatalf("first EnsureDirs() error = %v", err)
	}
	if err := EnsureDirs(); err != nil {
		t.Fatalf("second EnsureDirs() error = %v", err)
	}
}
