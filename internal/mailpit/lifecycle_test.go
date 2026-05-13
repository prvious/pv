package mailpit

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
)

func TestUpdate_NotInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	client := &http.Client{}
	err := Update(client, DefaultVersion())
	if err == nil {
		t.Fatal("expected error when service is not installed")
	}
	if !strings.Contains(err.Error(), "not installed") {
		t.Errorf("expected not-installed error; got %q", err)
	}
}

// TestUninstall_BinaryAlreadyRemoved verifies that an idempotent retry
// after a previous run that left the registry intact but removed the
// binary file completes successfully.
func TestUninstall_BinaryAlreadyRemoved(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(DefaultVersion(), false); err != nil {
		t.Fatalf("Uninstall with no binary file should succeed: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Versions[DefaultVersion()]; ok {
		t.Error("version state should be removed after uninstall")
	}
}

// TestUninstall_DeleteData verifies that --force/data-deletion actually
// wipes the data directory. This is the irreversible postgres-style
// :uninstall semantic; a regression here would silently spare user data
// the user explicitly asked to be deleted.
func TestUninstall_DeleteData(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.ServiceDataDir("mail", "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "mailpit.db")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(DefaultVersion(), true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); !os.IsNotExist(err) {
		t.Errorf("data directory must be deleted; sentinel still exists (err=%v)", err)
	}
}

func TestApplyFallbacksToLinkedProjects_RewritesEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	if err := os.WriteFile(envPath, []byte("MAIL_MAILER=smtp\n"), 0o644); err != nil {
		t.Fatalf("write .env: %v", err)
	}

	reg := &registry.Registry{
		Projects: []registry.Project{
			{
				Name:     "myapp",
				Path:     projectDir,
				Type:     "laravel",
				Services: &registry.ProjectServices{Mail: "latest"},
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
	if got := env["MAIL_MAILER"]; got != "log" {
		t.Errorf("MAIL_MAILER = %q, want %q", got, "log")
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
		t.Fatalf("WantedVersions should ignore not-installed mailpit, got %v", versions)
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
		t.Fatal("expected non-latest mailpit version to fail")
	}
}

func TestUninstall_KeepsDataDirByDefault(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.ServiceDataDir("mail", "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "mailpit.db")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	if err := Uninstall(DefaultVersion(), false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); err != nil {
		t.Errorf("data directory must be preserved without --force; sentinel missing: %v", err)
	}
}

func TestEnvVars_RejectsInvalidVersion(t *testing.T) {
	_, err := EnvVars("bad-version", "anyproject")
	if err == nil {
		t.Fatal("EnvVars: expected error for invalid version")
	}
}

func TestBuildSupervisorProcess_RejectsInvalidVersion(t *testing.T) {
	_, err := BuildSupervisorProcess("bad-version")
	if err == nil {
		t.Fatal("BuildSupervisorProcess: expected error for invalid version")
	}
}
