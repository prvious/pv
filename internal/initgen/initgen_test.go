package initgen

import (
	"strings"
	"testing"

	"gopkg.in/yaml.v3"

	"github.com/prvious/pv/internal/config"
)

// parseGenerated rounds the generated YAML through the actual pv.yml
// parser. This catches malformed output, wrong field names, and yaml
// syntax errors in one assertion.
func parseGenerated(t *testing.T, body string) *config.ProjectConfig {
	t.Helper()
	var cfg config.ProjectConfig
	if err := yaml.Unmarshal([]byte(body), &cfg); err != nil {
		t.Fatalf("Unmarshal generated pv.yml: %v\n--- body ---\n%s", err, body)
	}
	return &cfg
}

func TestGenerate_LaravelWithPostgres(t *testing.T) {
	body := Generate(Options{
		ProjectType: "laravel",
		ProjectName: "myapp",
		PHP:         "8.4",
		Postgres:    "18",
	})
	cfg := parseGenerated(t, body)

	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	if cfg.Postgresql == nil {
		t.Fatal("Postgresql is nil, want declared")
	}
	if cfg.Postgresql.Version != "18" {
		t.Errorf("Postgresql.Version = %q, want %q", cfg.Postgresql.Version, "18")
	}
	if got := cfg.Postgresql.Env["DB_DATABASE"]; got != "myapp" {
		t.Errorf("DB_DATABASE = %q, want %q", got, "myapp")
	}
	if got := cfg.Postgresql.Env["DB_HOST"]; got != "{{ .host }}" {
		t.Errorf("DB_HOST = %q, want template", got)
	}
	if got := cfg.Env["APP_URL"]; got != "{{ .site_url }}" {
		t.Errorf("APP_URL = %q, want template", got)
	}

	joined := strings.Join(cfg.Setup, "\n")
	for _, want := range []string{"cp .env.example .env", "pv postgres:db:create myapp", "composer install", "php artisan key:generate", "php artisan migrate"} {
		if !strings.Contains(joined, want) {
			t.Errorf("setup missing %q\nsetup: %v", want, cfg.Setup)
		}
	}
}

func TestGenerate_LaravelWithMysql(t *testing.T) {
	body := Generate(Options{
		ProjectType: "laravel",
		ProjectName: "myapp",
		PHP:         "8.4",
		Mysql:       "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.Mysql == nil || cfg.Mysql.Version != "8.4" {
		t.Fatalf("Mysql = %+v, want version 8.4", cfg.Mysql)
	}
	if cfg.Postgresql != nil {
		t.Errorf("Postgresql should be nil when only Mysql is requested")
	}
	if got := cfg.Mysql.Env["DB_CONNECTION"]; got != "mysql" {
		t.Errorf("DB_CONNECTION = %q, want mysql", got)
	}
	joined := strings.Join(cfg.Setup, "\n")
	if !strings.Contains(joined, "pv mysql:db:create myapp") {
		t.Errorf("setup missing mysql db create:\n%v", cfg.Setup)
	}
}

func TestGenerate_LaravelWithoutDB(t *testing.T) {
	body := Generate(Options{
		ProjectType: "laravel",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.Postgresql != nil || cfg.Mysql != nil {
		t.Errorf("No DB block expected; got Postgresql=%v Mysql=%v", cfg.Postgresql, cfg.Mysql)
	}
	joined := strings.Join(cfg.Setup, "\n")
	if strings.Contains(joined, "migrate") {
		t.Errorf("setup should not include migrate when no DB:\n%v", cfg.Setup)
	}
	if strings.Contains(joined, "db:create") {
		t.Errorf("setup should not include db:create when no DB:\n%v", cfg.Setup)
	}
}

func TestGenerate_PHP(t *testing.T) {
	body := Generate(Options{
		ProjectType: "php",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	joined := strings.Join(cfg.Setup, "\n")
	if !strings.Contains(joined, "composer install") {
		t.Errorf("setup missing composer install:\n%v", cfg.Setup)
	}
	if strings.Contains(joined, "artisan") {
		t.Errorf("generic PHP setup should not reference artisan:\n%v", cfg.Setup)
	}
}

func TestGenerate_Static(t *testing.T) {
	body := Generate(Options{
		ProjectType: "static",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	if len(cfg.Setup) != 0 {
		t.Errorf("static project should have empty setup, got: %v", cfg.Setup)
	}
}

func TestGenerate_Unknown(t *testing.T) {
	body := Generate(Options{
		ProjectType: "",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	if len(cfg.Setup) != 0 {
		t.Errorf("unknown project should have empty setup, got: %v", cfg.Setup)
	}
}

func TestGenerate_OctaneSameAsLaravel(t *testing.T) {
	octane := Generate(Options{ProjectType: "laravel-octane", ProjectName: "myapp", PHP: "8.4", Postgres: "18"})
	laravel := Generate(Options{ProjectType: "laravel", ProjectName: "myapp", PHP: "8.4", Postgres: "18"})
	if octane != laravel {
		t.Errorf("laravel-octane should produce identical output to laravel (octane install is user-controlled, not auto-templated)")
	}
}
