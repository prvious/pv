package phpenv

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func scaffold(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}
	s := config.DefaultSettings()
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}
	return home
}

func installFakeVersion(t *testing.T, version string) {
	t.Helper()
	dir := config.PhpVersionDir(version)
	if err := os.MkdirAll(dir, 0755); err != nil {
		t.Fatal(err)
	}
	// Create fake frankenphp and php binaries.
	for _, name := range []string{"frankenphp", "php"} {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
}

func TestInstalledVersions_Empty(t *testing.T) {
	scaffold(t)

	versions, err := InstalledVersions()
	if err != nil {
		t.Fatalf("InstalledVersions() error = %v", err)
	}
	if len(versions) != 0 {
		t.Errorf("expected empty list, got %v", versions)
	}
}

func TestInstalledVersions_WithVersions(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.5")
	installFakeVersion(t, "8.4")

	versions, err := InstalledVersions()
	if err != nil {
		t.Fatalf("InstalledVersions() error = %v", err)
	}
	if len(versions) != 3 {
		t.Fatalf("expected 3 versions, got %d", len(versions))
	}
	// Should be sorted.
	if versions[0] != "8.3" || versions[1] != "8.4" || versions[2] != "8.5" {
		t.Errorf("expected [8.3 8.4 8.5], got %v", versions)
	}
}

func TestIsInstalled(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")

	if !IsInstalled("8.4") {
		t.Error("expected 8.4 to be installed")
	}
	if IsInstalled("8.3") {
		t.Error("expected 8.3 to not be installed")
	}
}

func TestSetGlobal_Success(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")

	if err := SetGlobal("8.4"); err != nil {
		t.Fatalf("SetGlobal() error = %v", err)
	}

	v, err := GlobalVersion()
	if err != nil {
		t.Fatalf("GlobalVersion() error = %v", err)
	}
	if v != "8.4" {
		t.Errorf("GlobalVersion() = %q, want %q", v, "8.4")
	}

	// Verify symlinks.
	binDir := config.BinDir()
	for _, name := range []string{"frankenphp", "php"} {
		link := filepath.Join(binDir, name)
		target, err := os.Readlink(link)
		if err != nil {
			t.Errorf("expected symlink %s: %v", name, err)
			continue
		}
		expected := filepath.Join(config.PhpVersionDir("8.4"), name)
		if target != expected {
			t.Errorf("symlink %s → %s, want %s", name, target, expected)
		}
	}
}

func TestSetGlobal_NotInstalled(t *testing.T) {
	scaffold(t)

	if err := SetGlobal("9.9"); err == nil {
		t.Error("expected error for uninstalled version")
	}
}

func TestGlobalVersion_NotSet(t *testing.T) {
	scaffold(t)

	_, err := GlobalVersion()
	if err == nil {
		t.Error("expected error when no global version set")
	}
}

func TestRemove_Success(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	if err := Remove("8.3"); err != nil {
		t.Fatalf("Remove() error = %v", err)
	}

	if IsInstalled("8.3") {
		t.Error("expected 8.3 to be removed")
	}
}

func TestRemove_CannotRemoveGlobal(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")
	if err := SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	if err := Remove("8.4"); err == nil {
		t.Error("expected error when removing global version")
	}
}

func TestRemove_NotInstalled(t *testing.T) {
	scaffold(t)

	if err := Remove("9.9"); err == nil {
		t.Error("expected error for uninstalled version")
	}
}
