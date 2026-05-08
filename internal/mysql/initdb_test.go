package mysql

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func buildFakeInitdb(t *testing.T, version string) {
	t.Helper()
	bin := config.MysqlBinDir(version)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "mysqld"), filepath.Join("testdata", "fake-initdb.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("go build fake-initdb: %v\n%s", err, out)
	}
}

func TestRunInitdb_FreshDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	buildFakeInitdb(t, "8.4")
	if err := RunInitdb("8.4"); err != nil {
		t.Fatalf("RunInitdb: %v", err)
	}
	autoCnf := filepath.Join(config.MysqlDataDir("8.4"), "auto.cnf")
	if _, err := os.Stat(autoCnf); err != nil {
		t.Errorf("auto.cnf not created: %v", err)
	}
}

func TestRunInitdb_AlreadyInitialized_NoOp(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := config.MysqlDataDir("8.4")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "auto.cnf"), []byte("[auto]\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	// fake mysqld is NOT installed; if RunInitdb tried to invoke it, it'd fail.
	if err := RunInitdb("8.4"); err != nil {
		t.Errorf("RunInitdb on initialized dir should be a no-op, got: %v", err)
	}
}
