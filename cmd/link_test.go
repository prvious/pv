package cmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

func writeDefaultSettings(t *testing.T) {
	t.Helper()
	s := config.DefaultSettings()
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}
}

// newLinkCmd builds a fresh link command not tied to the package-level rootCmd.
func newLinkCmd() *cobra.Command {
	var name string

	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	link := &cobra.Command{
		Use:  "link [path]",
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			// Sync local flag → package-level var before delegating.
			linkName = name
			return linkCmd.RunE(cmd, args)
		},
	}
	link.Flags().StringVar(&name, "name", "", "Custom name for the project")
	root.AddCommand(link)
	return root
}

func TestLink_ExplicitPathAndName(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := t.TempDir()

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if len(reg.List()) != 1 {
		t.Fatalf("expected 1 project, got %d", len(reg.List()))
	}

	absPath, _ := filepath.Abs(projDir)
	p := reg.Find("myapp")
	if p == nil {
		t.Fatal("project 'myapp' not found in registry")
	}
	if p.Path != absPath {
		t.Errorf("path = %q, want %q", p.Path, absPath)
	}
}

func TestLink_NonExistentPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", "/does/not/exist"})
	if err := cmd.Execute(); err == nil {
		t.Fatal("expected error for non-existent path, got nil")
	}
}

func TestLink_FileNotDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	f := filepath.Join(t.TempDir(), "file.txt")
	if err := os.WriteFile(f, []byte("hi"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", f})
	if err := cmd.Execute(); err == nil {
		t.Fatal("expected error for file path, got nil")
	}
}

func TestLink_RelinkPreservesServices(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := t.TempDir()

	// First link.
	cmd1 := newLinkCmd()
	cmd1.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd1.Execute(); err != nil {
		t.Fatalf("first link error = %v", err)
	}

	// Manually add services to the registry entry to simulate bound services.
	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry: %v", err)
	}
	p := reg.Find("myapp")
	if p == nil {
		t.Fatal("expected project myapp in registry")
	}
	p.Services = &registry.ProjectServices{MySQL: "8.4"}
	p.Databases = []string{"myapp"}
	if err := reg.Save(); err != nil {
		t.Fatalf("save registry: %v", err)
	}

	// Re-link the same project — should succeed, not error.
	cmd2 := newLinkCmd()
	cmd2.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd2.Execute(); err != nil {
		t.Fatalf("re-link should succeed, got error: %v", err)
	}

	// Verify services and databases were preserved.
	reg2, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry after relink: %v", err)
	}
	if len(reg2.List()) != 1 {
		t.Fatalf("expected 1 project after relink, got %d", len(reg2.List()))
	}
	p2 := reg2.Find("myapp")
	if p2 == nil {
		t.Fatal("expected project myapp in registry after relink")
	}
	if p2.Services == nil || p2.Services.MySQL != "8.4" {
		t.Errorf("expected MySQL service preserved, got services=%v", p2.Services)
	}
	if len(p2.Databases) != 1 || p2.Databases[0] != "myapp" {
		t.Errorf("expected databases preserved, got %v", p2.Databases)
	}
}

func TestLink_RelinkOverwritesStaleAliases(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := t.TempDir()

	// First link with two aliases in pv.yml.
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"),
		[]byte("php: \"8.4\"\naliases:\n  - admin.myapp.test\n  - api.myapp.test\n"),
		0o644); err != nil {
		t.Fatal(err)
	}

	cmd1 := newLinkCmd()
	cmd1.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd1.Execute(); err != nil {
		t.Fatalf("first link error = %v", err)
	}

	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry: %v", err)
	}
	p := reg.Find("myapp")
	if p == nil {
		t.Fatal("project 'myapp' not found in registry after first link")
	}
	if len(p.Aliases) != 2 {
		t.Errorf("after first link: Aliases = %v, want 2 entries", p.Aliases)
	}

	// Rewrite pv.yml with only one alias.
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"),
		[]byte("php: \"8.4\"\naliases:\n  - admin.myapp.test\n"),
		0o644); err != nil {
		t.Fatal(err)
	}

	// Re-link — should overwrite Aliases wholesale.
	cmd2 := newLinkCmd()
	cmd2.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd2.Execute(); err != nil {
		t.Fatalf("re-link error = %v", err)
	}

	reg2, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry after relink: %v", err)
	}
	p2 := reg2.Find("myapp")
	if p2 == nil {
		t.Fatal("project 'myapp' not found in registry after relink")
	}
	if len(p2.Aliases) != 1 || p2.Aliases[0] != "admin.myapp.test" {
		t.Errorf("after relink: Aliases = %v, want [admin.myapp.test]", p2.Aliases)
	}
}

func TestLink_RelinkUpdatesPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir1 := t.TempDir()
	projDir2 := t.TempDir()

	// First link to projDir1.
	cmd1 := newLinkCmd()
	cmd1.SetArgs([]string{"link", projDir1, "--name", "myapp"})
	if err := cmd1.Execute(); err != nil {
		t.Fatalf("first link error = %v", err)
	}

	// Re-link to projDir2.
	cmd2 := newLinkCmd()
	cmd2.SetArgs([]string{"link", projDir2, "--name", "myapp"})
	if err := cmd2.Execute(); err != nil {
		t.Fatalf("re-link error = %v", err)
	}

	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry: %v", err)
	}
	if len(reg.List()) != 1 {
		t.Fatalf("expected 1 project, got %d", len(reg.List()))
	}
	p := reg.Find("myapp")
	if p == nil {
		t.Fatal("expected project myapp in registry")
	}

	absPath2, _ := filepath.Abs(projDir2)
	if p.Path != absPath2 {
		t.Errorf("path = %q, want %q", p.Path, absPath2)
	}
}

func TestLink_DetectsLaravel(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := filepath.Join(t.TempDir(), "mylaravel")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}
	composerJSON := `{"require":{"laravel/framework":"^11.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composerJSON), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "laratest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("laratest")
	if p == nil {
		t.Fatal("project not found")
	}
	if p.Type != "laravel" {
		t.Errorf("Type = %q, want %q", p.Type, "laravel")
	}
}

func TestLink_DetectsStatic(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := filepath.Join(t.TempDir(), "mystatic")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "index.html"), []byte("<html></html>"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "statictest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("statictest")
	if p == nil {
		t.Fatal("project not found")
	}
	if p.Type != "static" {
		t.Errorf("Type = %q, want %q", p.Type, "static")
	}
}

func TestLink_DetectsEmptyAsUnknown(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := t.TempDir()

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "emptytest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("emptytest")
	if p == nil {
		t.Fatal("project not found")
	}
	if p.Type != "" {
		t.Errorf("Type = %q, want %q", p.Type, "")
	}
}

func TestLink_DefaultsToBasename(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := filepath.Join(t.TempDir(), "cool-project")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("cool-project")
	if p == nil {
		t.Fatal("expected project named 'cool-project'")
	}
}

func TestLink_CreatesCaddySnippetForLaravel(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := filepath.Join(t.TempDir(), "laravelcaddy")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}
	composerJSON := `{"require":{"laravel/framework":"^11.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composerJSON), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "laravelcaddy"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	siteFile := filepath.Join(config.SitesDir(), "laravelcaddy.caddy")
	data, err := os.ReadFile(siteFile)
	if err != nil {
		t.Fatalf("expected site config to exist: %v", err)
	}
	content := string(data)
	if !strings.Contains(content, "laravelcaddy.test {") {
		t.Errorf("expected domain in snippet, got:\n%s", content)
	}
	if !strings.Contains(content, "php_server") {
		t.Errorf("expected php_server in snippet, got:\n%s", content)
	}
}

func TestLink_NoCaddySnippetForUnknown(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := t.TempDir()

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "emptyproj"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	siteFile := filepath.Join(config.SitesDir(), "emptyproj.caddy")
	if _, err := os.Stat(siteFile); !os.IsNotExist(err) {
		t.Error("expected no .caddy file for unknown project type")
	}
}

func TestLink_CreatesCaddyfile(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := filepath.Join(t.TempDir(), "caddyfiletest")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "index.html"), []byte("<html></html>"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "caddyfiletest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	data, err := os.ReadFile(config.CaddyfilePath())
	if err != nil {
		t.Fatalf("expected Caddyfile to exist: %v", err)
	}
	content := string(data)
	if !strings.Contains(content, "frankenphp") {
		t.Error("expected 'frankenphp' in Caddyfile")
	}
	if !strings.Contains(content, "import sites/*") {
		t.Error("expected 'import sites/*' in Caddyfile")
	}
}

// scaffoldLaravelProject creates a minimal Laravel project directory for testing.
// It creates composer.json, .env.example, and a vendor/ dir (to prevent
// ComposerInstallStep from trying to run the real composer binary).
func scaffoldLaravelProject(t *testing.T, name string) string {
	t.Helper()
	projDir := filepath.Join(t.TempDir(), name)
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}
	composerJSON := `{"require":{"laravel/framework":"^11.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composerJSON), 0644); err != nil {
		t.Fatal(err)
	}
	envExample := "APP_NAME=Laravel\nAPP_KEY=\nAPP_URL=http://localhost\nDB_CONNECTION=sqlite\n"
	if err := os.WriteFile(filepath.Join(projDir, ".env.example"), []byte(envExample), 0644); err != nil {
		t.Fatal(err)
	}
	// Create vendor/ so ComposerInstallStep is skipped (no real composer in tests).
	if err := os.MkdirAll(filepath.Join(projDir, "vendor"), 0755); err != nil {
		t.Fatal(err)
	}
	return projDir
}

func TestLink_AutomationCopiesEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := scaffoldLaravelProject(t, "envtest")

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "envtest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	// .env should have been created by CopyEnvStep.
	envPath := filepath.Join(projDir, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		t.Fatal(".env was not created by automation pipeline")
	}

	env, err := projectenv.ReadDotEnv(envPath)
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	if env["APP_NAME"] != "Laravel" {
		t.Errorf("APP_NAME = %q, want %q", env["APP_NAME"], "Laravel")
	}
}

func TestLink_AutomationSetsAppURL(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := scaffoldLaravelProject(t, "urltest")

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "urltest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	env, err := projectenv.ReadDotEnv(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	want := "https://urltest.test"
	if env["APP_URL"] != want {
		t.Errorf("APP_URL = %q, want %q", env["APP_URL"], want)
	}
}

func TestLink_AutomationSkippedForNonLaravel(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := filepath.Join(t.TempDir(), "plainphp")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}
	// Write a .env.example but no laravel/framework in composer.json.
	if err := os.WriteFile(filepath.Join(projDir, ".env.example"), []byte("APP_KEY=\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "index.php"), []byte("<?php echo 'hi';"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "plainphp"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	// .env should NOT have been created — automation only runs for Laravel.
	if _, err := os.Stat(filepath.Join(projDir, ".env")); !os.IsNotExist(err) {
		t.Error(".env should not exist for non-Laravel project")
	}
}

func TestLink_AutomationRedetectsOctane(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := scaffoldLaravelProject(t, "octanetest")

	// Simulate Octane: add octane to composer.json and pre-create the worker file
	// (since artisan octane:install can't run in tests).
	composerJSON := `{"require":{"laravel/framework":"^11.0","laravel/octane":"^2.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composerJSON), 0644); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(projDir, "public"), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "public", "frankenphp-worker.php"), []byte("<?php"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "octanetest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	p := reg.Find("octanetest")
	if p == nil {
		t.Fatal("project not found")
	}
	if p.Type != "laravel-octane" {
		t.Errorf("Type = %q, want %q", p.Type, "laravel-octane")
	}
}

func TestLink_AutomationLoadsExistingEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := scaffoldLaravelProject(t, "existingenv")

	// Pre-create .env so CopyEnvStep is skipped but the file is loaded into context.
	envContent := "APP_NAME=Existing\nAPP_KEY=base64:existingkey\nAPP_URL=http://localhost\n"
	if err := os.WriteFile(filepath.Join(projDir, ".env"), []byte(envContent), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "existingenv"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	// SetAppURLStep should have updated APP_URL even with pre-existing .env.
	env, err := projectenv.ReadDotEnv(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	want := "https://existingenv.test"
	if env["APP_URL"] != want {
		t.Errorf("APP_URL = %q, want %q", env["APP_URL"], want)
	}
	// APP_KEY should remain unchanged (GenerateKeyStep skipped because key is set).
	if env["APP_KEY"] != "base64:existingkey" {
		t.Errorf("APP_KEY = %q, want %q", env["APP_KEY"], "base64:existingkey")
	}
}

func TestLink_AutoOffWarnsButLinksWithMissingVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := config.DefaultSettings()
	s.Defaults.PHP = "8.4"
	s.Automation.InstallPHPVersion = config.AutoOff
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}

	// Fake that 8.4 is installed (global).
	versionDir := config.PhpVersionDir("8.4")
	if err := os.MkdirAll(versionDir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(versionDir, "frankenphp"), []byte("fake"), 0755); err != nil {
		t.Fatal(err)
	}

	// Project requests 8.3, which is NOT installed.
	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "offtest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v (AutoOff should warn, not fail)", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("offtest")
	if p == nil {
		t.Fatal("project not found — AutoOff should still link")
	}
	if p.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", p.PHP, "8.3")
	}
}

func TestLink_SkipsInstallWhenNonGlobalVersionIsInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := config.DefaultSettings()
	s.Defaults.PHP = "8.4"
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}

	// Fake both versions installed.
	for _, ver := range []string{"8.4", "8.3"} {
		versionDir := config.PhpVersionDir(ver)
		if err := os.MkdirAll(versionDir, 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(versionDir, "frankenphp"), []byte("fake"), 0755); err != nil {
			t.Fatal(err)
		}
	}

	// Project requests 8.3 (non-global but installed).
	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "installedtest"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("installedtest")
	if p == nil {
		t.Fatal("project not found")
	}
	if p.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", p.PHP, "8.3")
	}
}

func TestLink_SkipsInstallWhenVersionIsGlobal(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := config.DefaultSettings()
	s.Defaults.PHP = "8.4"
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	// Fake that 8.4 is installed by creating the frankenphp binary.
	versionDir := config.PhpVersionDir("8.4")
	if err := os.MkdirAll(versionDir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(versionDir, "frankenphp"), []byte("fake"), 0755); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "globalver"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("globalver")
	if p == nil {
		t.Fatal("project not found")
	}
	if p.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", p.PHP, "8.4")
	}
}
