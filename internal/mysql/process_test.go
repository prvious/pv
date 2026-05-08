package mysql

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestBuildSupervisorProcess_NotInitialized_Errors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if _, err := BuildSupervisorProcess("8.4"); err == nil {
		t.Error("expected error when data dir not initialized")
	}
}

func TestBuildSupervisorProcess_HappyPath(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dataDir := config.MysqlDataDir("8.4")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\n"), 0o644)

	p, err := BuildSupervisorProcess("8.4")
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	if p.Name != "mysql-8.4" {
		t.Errorf("Name = %q, want mysql-8.4", p.Name)
	}
	if !strings.HasSuffix(p.Binary, "/mysql/8.4/bin/mysqld") {
		t.Errorf("Binary = %q, expected to end with /mysql/8.4/bin/mysqld", p.Binary)
	}
	// Expected flags — not order-sensitive.
	wantFlags := []string{
		"--datadir=" + dataDir,
		"--basedir=" + config.MysqlVersionDir("8.4"),
		"--port=33084",
		"--bind-address=127.0.0.1",
		"--socket=/tmp/pv-mysql-8.4.sock",
		"--pid-file=/tmp/pv-mysql-8.4.pid",
		"--log-error=" + config.MysqlLogPath("8.4"),
		"--mysqlx=OFF",
		"--skip-name-resolve",
	}
	for _, want := range wantFlags {
		found := false
		for _, got := range p.Args {
			if got == want {
				found = true
				break
			}
		}
		if !found {
			t.Errorf("missing arg %q in %v", want, p.Args)
		}
	}
	if !strings.HasSuffix(p.LogFile, "/logs/mysql-8.4.log") {
		t.Errorf("LogFile = %q", p.LogFile)
	}
	if p.Ready == nil {
		t.Error("Ready func is nil")
	}
}

func TestBuildSupervisorProcess_InvalidVersion_Errors(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// Create auto.cnf so the data-dir gate passes; the version itself is
	// the invalid bit (no port mapping).
	dataDir := config.MysqlDataDir("garbage")
	os.MkdirAll(dataDir, 0o755)
	os.WriteFile(filepath.Join(dataDir, "auto.cnf"), []byte("[auto]\n"), 0o644)
	if _, err := BuildSupervisorProcess("garbage"); err == nil {
		t.Error("expected error for invalid version")
	}
}
