package cmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

// newInitCmd builds a fresh cobra root + the package-level initCmd so
// tests can SetArgs without sharing state with other tests' rootCmd
// invocations. We reuse the same initCmd because its flags are bound
// to package-level vars; tests must reset those vars before each
// invocation.
func newInitCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	root.AddGroup(&cobra.Group{ID: "core", Title: "Core"})
	root.AddCommand(initCmd)
	return root
}

// resetInitFlags zeros the package-level flag vars so a previous
// test's "--force" / "--mysql" doesn't leak into the next one.
func resetInitFlags() {
	initForce = false
	initMysql = false
}

func TestInit_LaravelProject(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)
	resetInitFlags()

	projDir := t.TempDir()
	composer := `{"require":{"laravel/framework":"^11.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0o644); err != nil {
		t.Fatal(err)
	}

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	if err != nil {
		t.Fatalf("read pv.yml: %v", err)
	}
	s := string(body)
	for _, want := range []string{"php: ", "env:", "APP_URL", "setup:", "composer install", "php artisan key:generate"} {
		if !strings.Contains(s, want) {
			t.Errorf("pv.yml missing %q\n--- contents ---\n%s", want, s)
		}
	}
}

func TestInit_RefusesWhenPvYmlExists(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)
	resetInitFlags()

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, config.ProjectConfigFilename), []byte("php: \"8.4\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	err := cmd.Execute()
	if err == nil {
		t.Fatal("Execute: want error when pv.yml exists, got nil")
	}
	if !strings.Contains(err.Error(), "--force") {
		t.Errorf("err = %v, want it to suggest --force", err)
	}
}

func TestInit_ForceOverwrites(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)
	resetInitFlags()

	projDir := t.TempDir()
	existing := "php: \"7.4\"\n# this should be replaced\n"
	if err := os.WriteFile(filepath.Join(projDir, config.ProjectConfigFilename), []byte(existing), 0o644); err != nil {
		t.Fatal(err)
	}

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir, "--force"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init --force: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	if strings.Contains(s, "this should be replaced") {
		t.Errorf("pv.yml still contains the old content:\n%s", s)
	}
	if !strings.Contains(s, "php: ") {
		t.Errorf("pv.yml looks malformed:\n%s", s)
	}
}

func TestInit_GenericPHP(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)
	resetInitFlags()

	projDir := t.TempDir()
	composer := `{"require":{"monolog/monolog":"^3.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0o644); err != nil {
		t.Fatal(err)
	}

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init: %v", err)
	}

	body, _ := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	s := string(body)
	if !strings.Contains(s, "composer install") {
		t.Errorf("pv.yml should include composer install:\n%s", s)
	}
	if strings.Contains(s, "artisan") {
		t.Errorf("generic PHP pv.yml should NOT reference artisan:\n%s", s)
	}
}

func TestInit_UnknownProject(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)
	resetInitFlags()

	projDir := t.TempDir()
	// No markers; detection.Detect returns ""

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init: %v", err)
	}

	body, _ := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	s := string(body)
	if !strings.Contains(s, "php: ") {
		t.Errorf("pv.yml should have at least the php: field:\n%s", s)
	}
}

func TestResolveInitPath_StatError(t *testing.T) {
	_, err := resolveInitPath([]string{"/nonexistent/path/that/does/not/exist"})
	if err == nil {
		t.Fatal("resolveInitPath: want error for nonexistent path, got nil")
	}
}

func TestResolveInitPath_NotADirectory(t *testing.T) {
	dir := t.TempDir()
	filePath := filepath.Join(dir, "file.txt")
	if err := os.WriteFile(filePath, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := resolveInitPath([]string{filePath})
	if err == nil {
		t.Fatal("resolveInitPath: want error for file path, got nil")
	}
	if !strings.Contains(err.Error(), "not a directory") {
		t.Errorf("err = %v, want it to mention 'not a directory'", err)
	}
}

func TestResolveInit_MysqlFlagPrecedence(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Stage stub postgres 18 + mysql 8.4
	stagePostgresStub(t, "18")
	stageMysqlStub(t, "8.4")

	// preferMysql=false: postgres wins
	if got := resolveInitPostgres(false); got != "18" {
		t.Errorf("resolveInitPostgres(false) = %q, want 18", got)
	}
	if got := resolveInitMysql(false); got != "" {
		t.Errorf("resolveInitMysql(false) = %q, want empty (postgres wins)", got)
	}

	// preferMysql=true: mysql wins
	if got := resolveInitPostgres(true); got != "" {
		t.Errorf("resolveInitPostgres(true) = %q, want empty (mysql wins)", got)
	}
	if got := resolveInitMysql(true); got != "8.4" {
		t.Errorf("resolveInitMysql(true) = %q, want 8.4", got)
	}
}

// stagePostgresStub mirrors the helper in internal/automation/steps/.
// Creates ~/.pv/postgres/<major>/bin/postgres so postgres.InstalledMajors()
// reports the version. Caller must set HOME to a tempdir first.
func stagePostgresStub(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", bin, err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage postgres: %v", err)
	}
}

func stageMysqlStub(t *testing.T, version string) {
	t.Helper()
	bin := config.MysqlBinDir(version)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", bin, err)
	}
	if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte{}, 0o755); err != nil {
		t.Fatalf("stage mysqld: %v", err)
	}
}
