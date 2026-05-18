package postgres

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestBuildSupervisorProcess_NotInitialized_Errors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := BuildSupervisorProcess("17"); err == nil {
		t.Error("expected error when data dir not initialized")
	}
}

func TestBuildSupervisorProcess_HappyPath(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dataDir := config.ServiceDataDir("postgres", "17")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "PG_VERSION"), []byte("17"), 0o644)

	p, err := BuildSupervisorProcess("17")
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	if p.Name != "postgres-17" {
		t.Errorf("Name = %q, want postgres-17", p.Name)
	}
	if !strings.HasSuffix(p.Binary, "/postgres/17/bin/postgres") {
		t.Errorf("Binary = %q, expected to end with /postgres/17/bin/postgres", p.Binary)
	}
	wantArgs := []string{"-D", dataDir}
	if len(p.Args) != 2 || p.Args[0] != wantArgs[0] || p.Args[1] != wantArgs[1] {
		t.Errorf("Args = %v, want %v", p.Args, wantArgs)
	}
	if !strings.HasSuffix(p.LogFile, "/logs/postgres-17.log") {
		t.Errorf("LogFile = %q", p.LogFile)
	}
	if p.Ready == nil {
		t.Error("Ready func is nil")
	}
}
