package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadProjectConfig_ValidPHP(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestLoadProjectConfig_UnquotedValue(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: 8.4\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestLoadProjectConfig_SingleQuoted(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: '8.4'\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestLoadProjectConfig_WithComment(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.3\" # pinned for legacy\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.3")
	}
}

func TestLoadProjectConfig_EmptyPHP(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("# empty config\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "" {
		t.Errorf("PHP = %q, want empty", cfg.PHP)
	}
}

func TestLoadProjectConfig_InvalidYAML(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: [invalid\n"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := LoadProjectConfig(path)
	if err == nil {
		t.Error("expected error for invalid YAML")
	}
}

func TestLoadProjectConfig_FileNotFound(t *testing.T) {
	_, err := LoadProjectConfig("/nonexistent/pv.yml")
	if err == nil {
		t.Error("expected error for missing file")
	}
}

func TestLoadProjectConfig_ExtraWhitespace(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php:   \"8.4\"  \n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestFindProjectConfig_InCurrentDir(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	got := FindProjectConfig(dir)
	if got != path {
		t.Errorf("FindProjectConfig() = %q, want %q", got, path)
	}
}

func TestFindProjectConfig_InParentDir(t *testing.T) {
	parent := t.TempDir()
	child := filepath.Join(parent, "sub", "deep")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}

	pvPath := filepath.Join(parent, ProjectConfigFilename)
	if err := os.WriteFile(pvPath, []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	got := FindProjectConfig(child)
	if got != pvPath {
		t.Errorf("FindProjectConfig() = %q, want %q", got, pvPath)
	}
}

func TestFindProjectConfig_ClosestWins(t *testing.T) {
	parent := t.TempDir()
	child := filepath.Join(parent, "sub")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}

	if err := os.WriteFile(filepath.Join(parent, ProjectConfigFilename), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	childPath := filepath.Join(child, ProjectConfigFilename)
	if err := os.WriteFile(childPath, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	got := FindProjectConfig(child)
	if got != childPath {
		t.Errorf("FindProjectConfig() = %q, want %q (closest should win)", got, childPath)
	}
}

func TestFindProjectConfig_NotFound(t *testing.T) {
	dir := t.TempDir()

	got := FindProjectConfig(dir)
	if got != "" {
		t.Errorf("FindProjectConfig() = %q, want empty (no pv.yml)", got)
	}
}

func TestFindAndLoadProjectConfig_Found(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, ProjectConfigFilename), []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := FindAndLoadProjectConfig(dir)
	if err != nil {
		t.Fatalf("FindAndLoadProjectConfig() error = %v", err)
	}
	if cfg == nil {
		t.Fatal("FindAndLoadProjectConfig() returned nil config")
	}
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
}

func TestFindAndLoadProjectConfig_NotFound(t *testing.T) {
	dir := t.TempDir()

	cfg, err := FindAndLoadProjectConfig(dir)
	if err != nil {
		t.Fatalf("FindAndLoadProjectConfig() error = %v", err)
	}
	if cfg != nil {
		t.Errorf("FindAndLoadProjectConfig() = %v, want nil when no pv.yml", cfg)
	}
}

func TestFindAndLoadProjectConfig_WalksUp(t *testing.T) {
	parent := t.TempDir()
	child := filepath.Join(parent, "a", "b", "c")
	if err := os.MkdirAll(child, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(parent, ProjectConfigFilename), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := FindAndLoadProjectConfig(child)
	if err != nil {
		t.Fatalf("FindAndLoadProjectConfig() error = %v", err)
	}
	if cfg == nil {
		t.Fatal("FindAndLoadProjectConfig() returned nil config")
	}
	if cfg.PHP != "8.3" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.3")
	}
}

func TestFindAndLoadProjectConfig_InvalidYAML(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, ProjectConfigFilename), []byte("php: [broken\n"), 0644); err != nil {
		t.Fatal(err)
	}

	_, err := FindAndLoadProjectConfig(dir)
	if err == nil {
		t.Error("expected error for invalid YAML")
	}
}

func TestLoadProjectConfig_ParsesAliases(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := "php: \"8.4\"\naliases:\n  - admin.myapp.test\n  - api.myapp.test\n"
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	want := []string{"admin.myapp.test", "api.myapp.test"}
	if len(cfg.Aliases) != len(want) {
		t.Fatalf("Aliases len = %d, want %d", len(cfg.Aliases), len(want))
	}
	for i, a := range want {
		if cfg.Aliases[i] != a {
			t.Errorf("Aliases[%d] = %q, want %q", i, cfg.Aliases[i], a)
		}
	}
}

func TestLoadProjectConfig_ParsesTopLevelEnv(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := "php: \"8.4\"\nenv:\n  APP_URL: \"{{ .site_url }}\"\n  APP_NAME: MyApp\n"
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if got := cfg.Env["APP_URL"]; got != "{{ .site_url }}" {
		t.Errorf("Env[APP_URL] = %q, want %q", got, "{{ .site_url }}")
	}
	if got := cfg.Env["APP_NAME"]; got != "MyApp" {
		t.Errorf("Env[APP_NAME] = %q, want %q", got, "MyApp")
	}
}

func TestLoadProjectConfig_ParsesPostgresService(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
postgresql:
  version: "18"
  env:
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Postgresql == nil {
		t.Fatal("Postgresql is nil, want declared")
	}
	if cfg.Postgresql.Version != "18" {
		t.Errorf("Postgresql.Version = %q, want %q", cfg.Postgresql.Version, "18")
	}
	if got := cfg.Postgresql.Env["DB_HOST"]; got != "{{ .host }}" {
		t.Errorf("Postgresql.Env[DB_HOST] = %q, want %q", got, "{{ .host }}")
	}
}

func TestLoadProjectConfig_ParsesMysqlService(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
mysql:
  version: "8.0"
  env:
    DB_HOST: "{{ .host }}"
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Mysql == nil || cfg.Mysql.Version != "8.0" {
		t.Fatalf("Mysql = %+v, want version 8.0", cfg.Mysql)
	}
}

func TestLoadProjectConfig_ParsesRedisMailpitRustfs(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
redis:
  env:
    REDIS_HOST: "{{ .host }}"
mailpit:
  env:
    MAIL_HOST: "{{ .smtp_host }}"
rustfs:
  env:
    AWS_ENDPOINT: "{{ .endpoint }}"
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Redis == nil || cfg.Redis.Env["REDIS_HOST"] != "{{ .host }}" {
		t.Errorf("Redis = %+v, want REDIS_HOST templated", cfg.Redis)
	}
	if cfg.Mailpit == nil || cfg.Mailpit.Env["MAIL_HOST"] != "{{ .smtp_host }}" {
		t.Errorf("Mailpit = %+v, want MAIL_HOST templated", cfg.Mailpit)
	}
	if cfg.Rustfs == nil || cfg.Rustfs.Env["AWS_ENDPOINT"] != "{{ .endpoint }}" {
		t.Errorf("Rustfs = %+v, want AWS_ENDPOINT templated", cfg.Rustfs)
	}
}

func TestLoadProjectConfig_ParsesSetup(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
setup:
  - composer install
  - php artisan key:generate
  - php artisan migrate
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	want := []string{"composer install", "php artisan key:generate", "php artisan migrate"}
	if len(cfg.Setup) != len(want) {
		t.Fatalf("Setup len = %d, want %d", len(cfg.Setup), len(want))
	}
	for i, c := range want {
		if cfg.Setup[i] != c {
			t.Errorf("Setup[%d] = %q, want %q", i, cfg.Setup[i], c)
		}
	}
}

func TestLoadProjectConfig_OmittedServicesAreNil(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Postgresql != nil || cfg.Mysql != nil || cfg.Redis != nil ||
		cfg.Mailpit != nil || cfg.Rustfs != nil {
		t.Errorf("services should be nil when undeclared, got %+v / %+v / %+v / %+v / %+v",
			cfg.Postgresql, cfg.Mysql, cfg.Redis, cfg.Mailpit, cfg.Rustfs)
	}
	if len(cfg.Aliases) != 0 || len(cfg.Env) != 0 || len(cfg.Setup) != 0 {
		t.Errorf("optional slices/maps should be empty, got aliases=%v env=%v setup=%v",
			cfg.Aliases, cfg.Env, cfg.Setup)
	}
}
