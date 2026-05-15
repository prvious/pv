package postgres

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func buildFakeInitdb(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	cmd := exec.Command("go", "build", "-o", filepath.Join(bin, "initdb"), filepath.Join("testdata", "fake-initdb.go"))
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("go build fake-initdb: %v\n%s", err, out)
	}
}

func TestRunInitdb_FreshDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	buildFakeInitdb(t, "17")
	if err := RunInitdb("17"); err != nil {
		t.Fatalf("RunInitdb: %v", err)
	}
	pgVer := filepath.Join(config.ServiceDataDir("postgres", "17"), "PG_VERSION")
	if _, err := os.Stat(pgVer); err != nil {
		t.Errorf("PG_VERSION not created: %v", err)
	}
}

func TestRunInitdb_AlreadyInitialized_NoOp(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "PG_VERSION"), []byte("17"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	// fake initdb is NOT installed; if RunInitdb tried to invoke it, it'd fail.
	if err := RunInitdb("17"); err != nil {
		t.Errorf("RunInitdb on initialized dir should be a no-op, got: %v", err)
	}
}
