package cmd

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

func writeDefaultSettings(t *testing.T) {
	t.Helper()
	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}
	data, _ := json.Marshal(config.DefaultSettings())
	if err := os.WriteFile(config.SettingsPath(), data, 0644); err != nil {
		t.Fatalf("write settings error = %v", err)
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

func TestLink_DuplicateName(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := t.TempDir()

	cmd1 := newLinkCmd()
	cmd1.SetArgs([]string{"link", projDir, "--name", "dup"})
	if err := cmd1.Execute(); err != nil {
		t.Fatalf("first link error = %v", err)
	}

	projDir2 := t.TempDir()
	cmd2 := newLinkCmd()
	cmd2.SetArgs([]string{"link", projDir2, "--name", "dup"})
	if err := cmd2.Execute(); err == nil {
		t.Fatal("expected error for duplicate name, got nil")
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
