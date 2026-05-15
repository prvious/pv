package postgres

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func setupFakeInstall(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	dataDir := config.ServiceDataDir("postgres", major)
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644)
	logDir := config.LogsDir()
	os.MkdirAll(logDir, 0o755)
	os.WriteFile(config.PostgresLogPath(major), []byte("log"), 0o644)
	_ = SetWanted(major, "running")
	vs, _ := binaries.LoadVersions()
	vs.Set("postgres-"+major, "17.5")
	_ = vs.Save()
}

func TestUninstall_RemovesEverything(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeInstall(t, "17")
	if err := Uninstall("17"); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(config.PostgresVersionDir("17")); !os.IsNotExist(err) {
		t.Errorf("version dir not removed: %v", err)
	}
	if _, err := os.Stat(config.ServiceDataDir("postgres", "17")); !os.IsNotExist(err) {
		t.Errorf("data dir not removed: %v", err)
	}
	if _, err := os.Stat(config.PostgresLogPath("17")); !os.IsNotExist(err) {
		t.Errorf("log not removed: %v", err)
	}
	st, _ := LoadState()
	if _, ok := st.Majors["17"]; ok {
		t.Error("state entry not removed")
	}
	vs, _ := binaries.LoadVersions()
	if got := vs.Get("postgres-17"); got != "" {
		t.Errorf("version entry not removed: %q", got)
	}
}

func TestUninstall_Missing_NoOp(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Uninstall("17"); err != nil {
		t.Errorf("Uninstall on missing major should be a no-op, got: %v", err)
	}
}
