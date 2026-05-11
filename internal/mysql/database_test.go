package mysql

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// TestCreateDatabase_InvokesBundledMysqlClient stages a fake `mysql` client
// at ~/.pv/mysql/<version>/bin/mysql that echoes its argv to a sidecar log,
// then asserts CreateDatabase shelled out to the absolute path with the
// expected args (socket, user, --execute SQL).
func TestCreateDatabase_InvokesBundledMysqlClient(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	binDir := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	logPath := filepath.Join(t.TempDir(), "argv.log")
	stub := "#!/bin/sh\nprintf '%s\\n' \"$@\" > " + logPath + "\nexit 0\n"
	if err := os.WriteFile(filepath.Join(binDir, "mysql"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}

	if err := CreateDatabase("8.4", "my-app"); err != nil {
		t.Fatalf("CreateDatabase: %v", err)
	}

	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("read argv log: %v", err)
	}
	body := string(data)
	wantSubs := []string{
		"--socket=/tmp/pv-mysql-8.4.sock",
		"-u",
		"root",
		"-e",
		"CREATE DATABASE IF NOT EXISTS `my-app`",
	}
	for _, w := range wantSubs {
		if !strings.Contains(body, w) {
			t.Errorf("argv missing %q\nfull argv:\n%s", w, body)
		}
	}
}

// TestCreateDatabase_BackquoteEscapesIdentifier ensures dots/hyphens in
// the database name are wrapped in backquotes — bare names with these
// characters are invalid SQL identifiers and would otherwise raise a
// syntax error.
func TestCreateDatabase_BackquoteEscapesIdentifier(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	binDir := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	logPath := filepath.Join(t.TempDir(), "argv.log")
	stub := "#!/bin/sh\nprintf '%s\\n' \"$@\" > " + logPath + "\nexit 0\n"
	if err := os.WriteFile(filepath.Join(binDir, "mysql"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}

	if err := CreateDatabase("8.4", "my.weird-name"); err != nil {
		t.Fatalf("CreateDatabase: %v", err)
	}
	data, _ := os.ReadFile(logPath)
	if !strings.Contains(string(data), "`my.weird-name`") {
		t.Errorf("identifier not backquoted, got argv:\n%s", string(data))
	}
}

// TestDropDatabase_InvokesBundledMysqlClient stages a fake `mysql` client
// at ~/.pv/mysql/<version>/bin/mysql that echoes its argv to a sidecar log,
// then asserts DropDatabase shelled out to the absolute path with the
// expected args (socket, user, --execute SQL with IF EXISTS).
func TestDropDatabase_InvokesBundledMysqlClient(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	binDir := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	logPath := filepath.Join(t.TempDir(), "argv.log")
	stub := "#!/bin/sh\nprintf '%s\\n' \"$@\" > " + logPath + "\nexit 0\n"
	if err := os.WriteFile(filepath.Join(binDir, "mysql"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}

	if err := DropDatabase("8.4", "my-app"); err != nil {
		t.Fatalf("DropDatabase: %v", err)
	}

	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("read argv log: %v", err)
	}
	body := string(data)
	wantSubs := []string{
		"--socket=/tmp/pv-mysql-8.4.sock",
		"-u",
		"root",
		"-e",
		"DROP DATABASE IF EXISTS `my-app`",
	}
	for _, w := range wantSubs {
		if !strings.Contains(body, w) {
			t.Errorf("argv missing %q\nfull argv:\n%s", w, body)
		}
	}
}
