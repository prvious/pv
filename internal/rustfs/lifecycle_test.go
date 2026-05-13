package rustfs

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
)

// TestUninstall_DeleteData verifies that --force/data-deletion actually
// wipes the data directory. This is the irreversible postgres-style
// :uninstall semantic; a regression here would silently spare user data
// the user explicitly asked to be deleted.
func TestUninstall_DeleteData(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.ServiceDataDir("s3", "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "buckets.json")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	if err := Uninstall(DefaultVersion(), true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); !os.IsNotExist(err) {
		t.Errorf("data directory must be deleted; sentinel still exists (err=%v)", err)
	}
}

func TestState_SetWantedWantedVersionsRemove(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted(DefaultVersion(), WantedRunning); err != nil {
		t.Fatalf("SetWanted running: %v", err)
	}
	versions, err := WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions: %v", err)
	}
	if len(versions) != 0 {
		t.Fatalf("WantedVersions should ignore not-installed rustfs, got %v", versions)
	}

	binPath := filepath.Join(config.InternalBinDir(), Binary().Name)
	if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	if err := os.WriteFile(binPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatalf("write fake binary: %v", err)
	}

	versions, err = WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions installed: %v", err)
	}
	if len(versions) != 1 || versions[0] != DefaultVersion() {
		t.Fatalf("WantedVersions = %v, want [latest]", versions)
	}

	if err := RemoveVersion(DefaultVersion()); err != nil {
		t.Fatalf("RemoveVersion: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Versions[DefaultVersion()]; ok {
		t.Fatalf("state still contains latest after RemoveVersion: %#v", st.Versions)
	}
}

func TestValidateVersion_RejectsNonLatest(t *testing.T) {
	if err := ValidateVersion("1.0.0"); err == nil {
		t.Fatal("expected non-latest rustfs version to fail")
	}
}

func TestApplyFallbacksToLinkedProjects_RewritesEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	if err := os.WriteFile(envPath, []byte("FILESYSTEM_DISK=s3\n"), 0o644); err != nil {
		t.Fatalf("write .env: %v", err)
	}

	reg := &registry.Registry{
		Projects: []registry.Project{
			{
				Name:     "myapp",
				Path:     projectDir,
				Type:     "laravel",
				Services: &registry.ProjectServices{S3: "latest"},
			},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save registry: %v", err)
	}

	settings := config.DefaultSettings()
	settings.Automation.ServiceFallback = config.AutoOn
	if err := settings.Save(); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	ApplyFallbacksToLinkedProjects(reg)

	env, err := projectenv.ReadDotEnv(envPath)
	if err != nil {
		t.Fatalf("read .env after fallback: %v", err)
	}
	if got := env["FILESYSTEM_DISK"]; got != "local" {
		t.Errorf("FILESYSTEM_DISK = %q, want %q", got, "local")
	}
}
