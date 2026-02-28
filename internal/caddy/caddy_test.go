package caddy

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func scaffold(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}
	s := config.DefaultSettings()
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}
	return home
}

func readSiteConfig(t *testing.T, name string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(config.SitesDir(), name+".caddy"))
	if err != nil {
		t.Fatalf("reading site config: %v", err)
	}
	return string(data)
}

func TestGenerateSiteConfig_LaravelOctane(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "octane-app", Path: projDir, Type: "laravel-octane"}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "octane-app")

	if !strings.Contains(content, "octane-app.test {") {
		t.Error("expected domain octane-app.test")
	}
	if !strings.Contains(content, "worker {") {
		t.Error("expected worker block")
	}
	if !strings.Contains(content, "frankenphp-worker.php") {
		t.Error("expected frankenphp-worker.php")
	}
	if !strings.Contains(content, "watch") {
		t.Error("expected watch directive")
	}
	if !strings.Contains(content, filepath.Join(projDir, "public")) {
		t.Errorf("expected root with /public, got:\n%s", content)
	}
}

func TestGenerateSiteConfig_Laravel(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "lara-app", Path: projDir, Type: "laravel"}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "lara-app")

	if !strings.Contains(content, "lara-app.test {") {
		t.Error("expected domain lara-app.test")
	}
	if !strings.Contains(content, "php_server") {
		t.Error("expected php_server")
	}
	if strings.Contains(content, "worker {") {
		t.Error("did not expect worker block for plain laravel")
	}
	if !strings.Contains(content, filepath.Join(projDir, "public")) {
		t.Errorf("expected root with /public, got:\n%s", content)
	}
}

func TestGenerateSiteConfig_PHPWithPublicDir(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(projDir, "public"), 0755); err != nil {
		t.Fatal(err)
	}
	p := registry.Project{Name: "php-app", Path: projDir, Type: "php"}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "php-app")

	if !strings.Contains(content, filepath.Join(projDir, "public")) {
		t.Errorf("expected root ending with /public, got:\n%s", content)
	}
}

func TestGenerateSiteConfig_PHPWithoutPublicDir(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "php-simple", Path: projDir, Type: "php"}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "php-simple")

	if !strings.Contains(content, "root * "+projDir) {
		t.Errorf("expected root = project path, got:\n%s", content)
	}
}

func TestGenerateSiteConfig_Static(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "my-site", Path: projDir, Type: "static"}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "my-site")

	if !strings.Contains(content, "file_server") {
		t.Error("expected file_server directive")
	}
	if strings.Contains(content, "php_server") {
		t.Error("did not expect php_server for static site")
	}
}

func TestGenerateSiteConfig_UnknownType(t *testing.T) {
	scaffold(t)

	p := registry.Project{Name: "unknown", Path: "/tmp/unknown", Type: ""}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	path := filepath.Join(config.SitesDir(), "unknown.caddy")
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Error("expected no file for unknown type")
	}
}

func TestGenerateSiteConfig_DomainName(t *testing.T) {
	scaffold(t)

	p := registry.Project{Name: "my-app", Path: t.TempDir(), Type: "static"}

	if err := GenerateSiteConfig(p); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "my-app")

	if !strings.Contains(content, "my-app.test {") {
		t.Errorf("expected 'my-app.test {' in output, got:\n%s", content)
	}
}

func TestRemoveSiteConfig_ExistingFile(t *testing.T) {
	scaffold(t)

	path := filepath.Join(config.SitesDir(), "removeme.caddy")
	if err := os.WriteFile(path, []byte("test"), 0644); err != nil {
		t.Fatal(err)
	}

	if err := RemoveSiteConfig("removeme"); err != nil {
		t.Fatalf("RemoveSiteConfig() error = %v", err)
	}

	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Error("expected file to be deleted")
	}
}

func TestRemoveSiteConfig_NonExistent(t *testing.T) {
	scaffold(t)

	if err := RemoveSiteConfig("doesnotexist"); err != nil {
		t.Fatalf("RemoveSiteConfig() error = %v, expected nil", err)
	}
}

func TestGenerateCaddyfile(t *testing.T) {
	scaffold(t)

	if err := GenerateCaddyfile(); err != nil {
		t.Fatalf("GenerateCaddyfile() error = %v", err)
	}

	data, err := os.ReadFile(config.CaddyfilePath())
	if err != nil {
		t.Fatalf("reading Caddyfile: %v", err)
	}
	content := string(data)

	if !strings.Contains(content, "frankenphp") {
		t.Error("expected 'frankenphp' in Caddyfile")
	}
	if !strings.Contains(content, "import sites/*") {
		t.Error("expected 'import sites/*' in Caddyfile")
	}
}

func TestGenerateAllSiteConfigs(t *testing.T) {
	scaffold(t)

	projects := []registry.Project{
		{Name: "app1", Path: t.TempDir(), Type: "laravel"},
		{Name: "app2", Path: t.TempDir(), Type: "static"},
		{Name: "app3", Path: t.TempDir(), Type: ""},
	}

	if err := GenerateAllSiteConfigs(projects); err != nil {
		t.Fatalf("GenerateAllSiteConfigs() error = %v", err)
	}

	// app1 and app2 should have config files
	for _, name := range []string{"app1", "app2"} {
		path := filepath.Join(config.SitesDir(), name+".caddy")
		if _, err := os.Stat(path); err != nil {
			t.Errorf("expected %s.caddy to exist", name)
		}
	}

	// app3 (unknown) should not have a config file
	path := filepath.Join(config.SitesDir(), "app3.caddy")
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Error("expected no file for unknown type project app3")
	}
}
