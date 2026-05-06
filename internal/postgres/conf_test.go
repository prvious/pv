package postgres

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func setupFakeDataDir(t *testing.T, major string) string {
	t.Helper()
	dir := config.ServiceDataDir("postgres", major)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "postgresql.conf"), []byte("# initdb default\n"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	return dir
}

func TestWriteOverrides(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeDataDir(t, "17")
	if err := WriteOverrides("17"); err != nil {
		t.Fatalf("WriteOverrides: %v", err)
	}
	got, err := os.ReadFile(filepath.Join(config.ServiceDataDir("postgres", "17"), "postgresql.conf"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"port = 54017",
		"listen_addresses = '127.0.0.1'",
		"unix_socket_directories = '/tmp/pv-postgres-17'",
		"fsync = on",
		"logging_collector = off",
	} {
		if !strings.Contains(string(got), want) {
			t.Errorf("missing %q in postgresql.conf:\n%s", want, got)
		}
	}
}

func TestWriteOverrides_Idempotent(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	setupFakeDataDir(t, "17")
	for i := 0; i < 3; i++ {
		if err := WriteOverrides("17"); err != nil {
			t.Fatalf("WriteOverrides #%d: %v", i, err)
		}
	}
	got, err := os.ReadFile(filepath.Join(config.ServiceDataDir("postgres", "17"), "postgresql.conf"))
	if err != nil {
		t.Fatal(err)
	}
	if c := strings.Count(string(got), "# pv-managed begin"); c != 1 {
		t.Errorf("expected 1 pv-managed block, got %d", c)
	}
}

func TestRewriteHBA(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := RewriteHBA("17"); err != nil {
		t.Fatalf("RewriteHBA: %v", err)
	}
	got, err := os.ReadFile(filepath.Join(dir, "pg_hba.conf"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"local   all             all                                     trust",
		"host    all             all             127.0.0.1/32            trust",
		"host    all             all             ::1/128                 trust",
	} {
		if !strings.Contains(string(got), want) {
			t.Errorf("missing %q in pg_hba.conf:\n%s", want, got)
		}
	}
}
