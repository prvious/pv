package caddy

import (
	"fmt"
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

func readVersionSiteConfig(t *testing.T, version, name string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(config.VersionSitesDir(version), name+".caddy"))
	if err != nil {
		t.Fatalf("reading version site config: %v", err)
	}
	return string(data)
}

func TestGenerateSiteConfig_LaravelOctane(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "octane-app", Path: projDir, Type: "laravel-octane"}

	if err := GenerateSiteConfig(p, ""); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "octane-app")

	if !strings.Contains(content, "octane-app.test, *.octane-app.test {") {
		t.Error("expected domain octane-app.test with wildcard")
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

func TestOctaneTemplate_WatchesAutoload(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "octane-autoload", Path: projDir, Type: "laravel-octane"}

	if err := GenerateSiteConfig(p, ""); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "octane-autoload")

	if !strings.Contains(content, "vendor/autoload.php") {
		t.Errorf("expected 'vendor/autoload.php' watch directive in Octane config, got:\n%s", content)
	}
}

func TestOctaneTemplate_WatchesAutoload_VersionSpecific(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "octane-ver", Path: projDir, Type: "laravel-octane", PHP: "8.3"}

	if err := GenerateSiteConfig(p, "8.4"); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readVersionSiteConfig(t, "8.3", "octane-ver")

	if !strings.Contains(content, "vendor/autoload.php") {
		t.Errorf("expected 'vendor/autoload.php' watch directive in version Octane config, got:\n%s", content)
	}
}

func TestVersionSiteConfig_HasWildcard(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "ver-app", Path: projDir, Type: "laravel", PHP: "8.3"}

	if err := GenerateSiteConfig(p, "8.4"); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readVersionSiteConfig(t, "8.3", "ver-app")

	if !strings.Contains(content, "http://ver-app.test, http://*.ver-app.test {") {
		t.Errorf("expected wildcard in version site config, got:\n%s", content)
	}
}

func TestGenerateSiteConfig_Laravel(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "lara-app", Path: projDir, Type: "laravel"}

	if err := GenerateSiteConfig(p, ""); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "lara-app")

	if !strings.Contains(content, "lara-app.test, *.lara-app.test {") {
		t.Error("expected domain lara-app.test with wildcard")
	}
	if !strings.Contains(content, "php_server") {
		t.Error("expected php_server")
	}
	if strings.Contains(content, "worker") {
		t.Error("did not expect worker directive for plain laravel (only Octane uses worker mode)")
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

	if err := GenerateSiteConfig(p, ""); err != nil {
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

	if err := GenerateSiteConfig(p, ""); err != nil {
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

	if err := GenerateSiteConfig(p, ""); err != nil {
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

	if err := GenerateSiteConfig(p, ""); err != nil {
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

	if err := GenerateSiteConfig(p, ""); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "my-app")

	if !strings.Contains(content, "my-app.test, *.my-app.test {") {
		t.Errorf("expected 'my-app.test, *.my-app.test {' in output, got:\n%s", content)
	}
}

// --- Multi-version tests ---

func TestGenerateSiteConfig_DirectOnGlobalVersion(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "app1", Path: projDir, Type: "laravel", PHP: "8.4"}

	if err := GenerateSiteConfig(p, "8.4"); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "app1")
	if !strings.Contains(content, "php_server") {
		t.Error("expected php_server for global version project")
	}
	if strings.Contains(content, "reverse_proxy") {
		t.Error("did not expect reverse_proxy for global version project")
	}
}

func TestGenerateSiteConfig_ProxyOnNonGlobalVersion(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	p := registry.Project{Name: "app2", Path: projDir, Type: "laravel", PHP: "8.3"}

	if err := GenerateSiteConfig(p, "8.4"); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	// Main sites/ should have reverse_proxy.
	content := readSiteConfig(t, "app2")
	if !strings.Contains(content, "reverse_proxy") {
		t.Error("expected reverse_proxy for non-global version project")
	}
	port := config.PortForVersion("8.3")
	if !strings.Contains(content, fmt.Sprintf("127.0.0.1:%d", port)) {
		t.Errorf("expected port %d in proxy config, got:\n%s", port, content)
	}

	// Version-specific dir should have php_server.
	vContent := readVersionSiteConfig(t, "8.3", "app2")
	if !strings.Contains(vContent, "php_server") {
		t.Error("expected php_server in version-specific config")
	}
	// Version config should NOT have tls internal.
	if strings.Contains(vContent, "tls internal") {
		t.Error("did not expect tls in version-specific config")
	}
}

func TestGenerateSiteConfig_StaticAlwaysDirect(t *testing.T) {
	scaffold(t)

	p := registry.Project{Name: "static-site", Path: t.TempDir(), Type: "static", PHP: "8.3"}

	if err := GenerateSiteConfig(p, "8.4"); err != nil {
		t.Fatalf("GenerateSiteConfig() error = %v", err)
	}

	content := readSiteConfig(t, "static-site")
	if strings.Contains(content, "reverse_proxy") {
		t.Error("static sites should never be proxied")
	}
	if !strings.Contains(content, "file_server") {
		t.Error("expected file_server for static site")
	}
}

func TestGenerateVersionCaddyfile(t *testing.T) {
	scaffold(t)

	if err := GenerateVersionCaddyfile("8.3"); err != nil {
		t.Fatalf("GenerateVersionCaddyfile() error = %v", err)
	}

	data, err := os.ReadFile(config.VersionCaddyfilePath("8.3"))
	if err != nil {
		t.Fatalf("reading version Caddyfile: %v", err)
	}
	content := string(data)

	if !strings.Contains(content, "frankenphp") {
		t.Error("expected frankenphp in version Caddyfile")
	}
	if !strings.Contains(content, "auto_https off") {
		t.Error("expected auto_https off")
	}
	if !strings.Contains(content, "admin off") {
		t.Error("expected admin off")
	}
	port := config.PortForVersion("8.3")
	if !strings.Contains(content, fmt.Sprintf("http_port %d", port)) {
		t.Errorf("expected http_port %d, got:\n%s", port, content)
	}
	if !strings.Contains(content, "import sites-8.3/*") {
		t.Error("expected import sites-8.3/*")
	}
	if !strings.Contains(content, "log {") {
		t.Error("expected 'log {' in version Caddyfile")
	}
	if !strings.Contains(content, "output file") {
		t.Error("expected 'output file' in version Caddyfile")
	}
	if !strings.Contains(content, "roll_size 10MiB") {
		t.Error("expected 'roll_size 10MiB' in version Caddyfile")
	}
	if !strings.Contains(content, config.CaddyLogPathForVersion("8.3")) {
		t.Errorf("expected log path %s in version Caddyfile", config.CaddyLogPathForVersion("8.3"))
	}
}

func TestActiveVersions(t *testing.T) {
	projects := []registry.Project{
		{Name: "app1", Type: "laravel", PHP: "8.4"},
		{Name: "app2", Type: "laravel", PHP: "8.3"},
		{Name: "app3", Type: "static", PHP: "8.3"},
		{Name: "app4", Type: "laravel"},
	}

	active := ActiveVersions(projects, "8.4")

	if !active["8.3"] {
		t.Error("expected 8.3 to be active")
	}
	if active["8.4"] {
		t.Error("did not expect global version 8.4 to be in active set")
	}
}

func TestGenerateAllConfigs(t *testing.T) {
	scaffold(t)

	projects := []registry.Project{
		{Name: "app1", Path: t.TempDir(), Type: "laravel", PHP: "8.4"},
		{Name: "app2", Path: t.TempDir(), Type: "laravel", PHP: "8.3"},
		{Name: "app3", Path: t.TempDir(), Type: "static"},
		{Name: "app4", Path: t.TempDir(), Type: ""},
	}

	if err := GenerateAllConfigs(projects, "8.4"); err != nil {
		t.Fatalf("GenerateAllConfigs() error = %v", err)
	}

	// app1 (global version) should have php_server in sites/.
	c1 := readSiteConfig(t, "app1")
	if !strings.Contains(c1, "php_server") {
		t.Error("app1 should have php_server")
	}

	// app2 (non-global) should have reverse_proxy in sites/ and php_server in sites-8.3/.
	c2 := readSiteConfig(t, "app2")
	if !strings.Contains(c2, "reverse_proxy") {
		t.Error("app2 should have reverse_proxy")
	}
	vc2 := readVersionSiteConfig(t, "8.3", "app2")
	if !strings.Contains(vc2, "php_server") {
		t.Error("app2 should have php_server in version config")
	}

	// app3 (static) should have file_server.
	c3 := readSiteConfig(t, "app3")
	if !strings.Contains(c3, "file_server") {
		t.Error("app3 should have file_server")
	}

	// app4 (unknown) should have no config.
	path := filepath.Join(config.SitesDir(), "app4.caddy")
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Error("expected no file for unknown type app4")
	}

	// Main Caddyfile should exist.
	if _, err := os.Stat(config.CaddyfilePath()); err != nil {
		t.Error("expected main Caddyfile to exist")
	}

	// Version Caddyfile for 8.3 should exist.
	if _, err := os.Stat(config.VersionCaddyfilePath("8.3")); err != nil {
		t.Error("expected php-8.3.Caddyfile to exist")
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

func TestRemoveSiteConfig_CleansVersionDirs(t *testing.T) {
	scaffold(t)

	// Create a config in a version-specific dir.
	vDir := config.VersionSitesDir("8.3")
	if err := os.MkdirAll(vDir, 0755); err != nil {
		t.Fatal(err)
	}
	vPath := filepath.Join(vDir, "myapp.caddy")
	if err := os.WriteFile(vPath, []byte("test"), 0644); err != nil {
		t.Fatal(err)
	}

	if err := RemoveSiteConfig("myapp"); err != nil {
		t.Fatalf("RemoveSiteConfig() error = %v", err)
	}

	if _, err := os.Stat(vPath); !os.IsNotExist(err) {
		t.Error("expected version-specific config to be deleted")
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
	if !strings.Contains(content, "local_certs") {
		t.Error("expected 'local_certs' in Caddyfile")
	}
	if !strings.Contains(content, "import sites/*") {
		t.Error("expected 'import sites/*' in Caddyfile")
	}
	if !strings.Contains(content, "log {") {
		t.Error("expected 'log {' in Caddyfile")
	}
	if !strings.Contains(content, "output file") {
		t.Error("expected 'output file' in Caddyfile")
	}
	if !strings.Contains(content, "roll_size 10MiB") {
		t.Error("expected 'roll_size 10MiB' in Caddyfile")
	}
	if !strings.Contains(content, config.CaddyLogPath()) {
		t.Errorf("expected log path %s in Caddyfile", config.CaddyLogPath())
	}
}

func TestGenerateServiceSiteConfigs(t *testing.T) {
	scaffold(t)

	binDir := config.InternalBinDir()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatalf("mkdir bin dir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(binDir, "rustfs"), []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatalf("write rustfs: %v", err)
	}
	if err := os.WriteFile(filepath.Join(binDir, "mailpit"), []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatalf("write mailpit: %v", err)
	}

	if err := GenerateServiceSiteConfigs(); err != nil {
		t.Fatalf("GenerateServiceSiteConfigs() error = %v", err)
	}

	// S3 has subdomain "s3", so _svc-s3.caddy should exist.
	svcPath := filepath.Join(config.SitesDir(), "_svc-s3.caddy")
	data, err := os.ReadFile(svcPath)
	if err != nil {
		t.Fatalf("expected _svc-s3.caddy to exist: %v", err)
	}
	content := string(data)
	if !strings.Contains(content, "s3.pv.test {") {
		t.Errorf("expected 's3.pv.test {' in output, got:\n%s", content)
	}
	if !strings.Contains(content, "reverse_proxy 127.0.0.1:9001") {
		t.Errorf("expected 'reverse_proxy 127.0.0.1:9001', got:\n%s", content)
	}
	if !strings.Contains(content, "tls internal") {
		t.Errorf("expected 'tls internal', got:\n%s", content)
	}

	// S3 API route: _svc-s3-api.caddy should also exist.
	apiPath := filepath.Join(config.SitesDir(), "_svc-s3-api.caddy")
	apiData, err := os.ReadFile(apiPath)
	if err != nil {
		t.Fatalf("expected _svc-s3-api.caddy to exist: %v", err)
	}
	apiContent := string(apiData)
	if !strings.Contains(apiContent, "s3-api.pv.test {") {
		t.Errorf("expected 's3-api.pv.test {' in output, got:\n%s", apiContent)
	}
	if !strings.Contains(apiContent, "reverse_proxy 127.0.0.1:9000") {
		t.Errorf("expected 'reverse_proxy 127.0.0.1:9000', got:\n%s", apiContent)
	}

	// Mailpit has subdomain "mail", so _svc-mail.caddy should exist.
	mailPath := filepath.Join(config.SitesDir(), "_svc-mail.caddy")
	mailData, err := os.ReadFile(mailPath)
	if err != nil {
		t.Fatalf("expected _svc-mail.caddy to exist: %v", err)
	}
	mailContent := string(mailData)
	if !strings.Contains(mailContent, "mail.pv.test {") {
		t.Errorf("expected 'mail.pv.test {' in output, got:\n%s", mailContent)
	}
	if !strings.Contains(mailContent, "reverse_proxy 127.0.0.1:8025") {
		t.Errorf("expected 'reverse_proxy 127.0.0.1:8025', got:\n%s", mailContent)
	}

	// Redis has no web routes, so only _svc-s3, _svc-s3-api and _svc-mail should exist.
	expected := map[string]bool{"_svc-s3.caddy": true, "_svc-s3-api.caddy": true, "_svc-mail.caddy": true}
	entries, _ := os.ReadDir(config.SitesDir())
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), "_svc-") && !expected[e.Name()] {
			t.Errorf("unexpected service config file: %s", e.Name())
		}
	}
}

func TestGenerateServiceSiteConfigs_Empty(t *testing.T) {
	scaffold(t)

	if err := GenerateServiceSiteConfigs(); err != nil {
		t.Fatalf("GenerateServiceSiteConfigs() error = %v", err)
	}

	// No _svc- files should exist.
	entries, _ := os.ReadDir(config.SitesDir())
	for _, e := range entries {
		if strings.HasPrefix(e.Name(), "_svc-") {
			t.Errorf("unexpected service config file: %s", e.Name())
		}
	}
}

func TestGenerateSiteConfig_IncludesAliases(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	proj := registry.Project{
		Name:    "myapp",
		Path:    projDir,
		Type:    "php",
		PHP:     "8.4",
		Aliases: []string{"admin.myapp.test", "api.myapp.test"},
	}
	if err := GenerateSiteConfig(proj, ""); err != nil {
		t.Fatalf("GenerateSiteConfig: %v", err)
	}
	cfg := readSiteConfig(t, "myapp")
	for _, want := range []string{"myapp.test", "*.myapp.test", "admin.myapp.test", "api.myapp.test"} {
		if !strings.Contains(cfg, want) {
			t.Errorf("config missing host %q\n%s", want, cfg)
		}
	}
}

func TestGenerateSiteConfig_NoAliasesPreservesLegacy(t *testing.T) {
	scaffold(t)

	projDir := t.TempDir()
	proj := registry.Project{
		Name: "myapp",
		Path: projDir,
		Type: "php",
		PHP:  "8.4",
	}
	if err := GenerateSiteConfig(proj, ""); err != nil {
		t.Fatalf("GenerateSiteConfig: %v", err)
	}
	cfg := readSiteConfig(t, "myapp")
	if !strings.Contains(cfg, "myapp.test, *.myapp.test {") {
		t.Errorf("legacy two-host line missing\n%s", cfg)
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
