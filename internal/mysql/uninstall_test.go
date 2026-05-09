package mysql

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func setupFakeInstall(t *testing.T, version string) {
	t.Helper()
	bin := config.MysqlBinDir(version)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte{}, 0o755); err != nil {
		t.Fatal(err)
	}
	dataDir := config.MysqlDataDir(version)
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\n"), 0o644)
	logDir := config.LogsDir()
	os.MkdirAll(logDir, 0o755)
	os.WriteFile(config.MysqlLogPath(version), []byte("log"), 0o644)
	_ = SetWanted(version, WantedRunning)
	vs, _ := binaries.LoadVersions()
	vs.Set("mysql-"+version, "8.4.3")
	_ = vs.Save()
}

func TestUninstall_KeepsDataDirByDefault(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeInstall(t, "8.4")
	if err := Uninstall("8.4", false); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(config.MysqlVersionDir("8.4")); !os.IsNotExist(err) {
		t.Errorf("version dir not removed: %v", err)
	}
	if _, err := os.Stat(config.MysqlDataDir("8.4")); err != nil {
		t.Errorf("data dir should be preserved without force, got: %v", err)
	}
	if _, err := os.Stat(config.MysqlLogPath("8.4")); !os.IsNotExist(err) {
		t.Errorf("log not removed: %v", err)
	}
	st, _ := LoadState()
	if _, ok := st.Versions["8.4"]; ok {
		t.Error("state entry not removed")
	}
	vs, _ := binaries.LoadVersions()
	if got := vs.Get("mysql-8.4"); got != "" {
		t.Errorf("version entry not removed: %q", got)
	}
}

func TestUninstall_ForceRemovesDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeInstall(t, "8.4")
	if err := Uninstall("8.4", true); err != nil {
		t.Fatalf("Uninstall force: %v", err)
	}
	if _, err := os.Stat(config.MysqlDataDir("8.4")); !os.IsNotExist(err) {
		t.Errorf("data dir not removed with force: %v", err)
	}
}

func TestUninstall_Missing_NoOp(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := Uninstall("8.4", false); err != nil {
		t.Errorf("Uninstall on missing version should be a no-op, got: %v", err)
	}
}
