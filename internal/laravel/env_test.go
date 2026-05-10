package laravel

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

func TestSmartEnvVars_Redis(t *testing.T) {
	bound := &registry.ProjectServices{Redis: true}
	vars := SmartEnvVars(bound)

	if len(vars) != 3 {
		t.Fatalf("expected 3 vars, got %d", len(vars))
	}
	if vars["CACHE_STORE"] != "redis" {
		t.Errorf("CACHE_STORE = %q, want %q", vars["CACHE_STORE"], "redis")
	}
	if vars["SESSION_DRIVER"] != "redis" {
		t.Errorf("SESSION_DRIVER = %q, want %q", vars["SESSION_DRIVER"], "redis")
	}
	if vars["QUEUE_CONNECTION"] != "redis" {
		t.Errorf("QUEUE_CONNECTION = %q, want %q", vars["QUEUE_CONNECTION"], "redis")
	}
}

func TestSmartEnvVars_S3(t *testing.T) {
	bound := &registry.ProjectServices{S3: true}
	vars := SmartEnvVars(bound)

	if len(vars) != 1 {
		t.Fatalf("expected 1 var, got %d", len(vars))
	}
	if vars["FILESYSTEM_DISK"] != "s3" {
		t.Errorf("FILESYSTEM_DISK = %q, want %q", vars["FILESYSTEM_DISK"], "s3")
	}
}

func TestSmartEnvVars_Mail(t *testing.T) {
	bound := &registry.ProjectServices{Mail: true}
	vars := SmartEnvVars(bound)

	if len(vars) != 1 {
		t.Fatalf("expected 1 var, got %d", len(vars))
	}
	if vars["MAIL_MAILER"] != "smtp" {
		t.Errorf("MAIL_MAILER = %q, want %q", vars["MAIL_MAILER"], "smtp")
	}
}

func TestSmartEnvVars_NoServices(t *testing.T) {
	bound := &registry.ProjectServices{}
	vars := SmartEnvVars(bound)

	if len(vars) != 0 {
		t.Errorf("expected empty map, got %d vars", len(vars))
	}
}

func TestSmartEnvVars_AllServices(t *testing.T) {
	bound := &registry.ProjectServices{
		Redis: true,
		S3:    true,
		Mail:  true,
	}
	vars := SmartEnvVars(bound)

	if len(vars) != 5 {
		t.Fatalf("expected 5 vars, got %d: %v", len(vars), vars)
	}
	expected := map[string]string{
		"CACHE_STORE":      "redis",
		"SESSION_DRIVER":   "redis",
		"QUEUE_CONNECTION": "redis",
		"FILESYSTEM_DISK":  "s3",
		"MAIL_MAILER":      "smtp",
	}
	for k, v := range expected {
		if vars[k] != v {
			t.Errorf("%s = %q, want %q", k, vars[k], v)
		}
	}
}

func TestFallbackMapping_Redis(t *testing.T) {
	rules := FallbackMapping("redis")
	if len(rules) != 3 {
		t.Fatalf("expected 3 rules, got %d", len(rules))
	}

	tests := map[string]FallbackRule{
		"CACHE_STORE":      {IfValue: "redis", ReplaceWith: "file"},
		"SESSION_DRIVER":   {IfValue: "redis", ReplaceWith: "file"},
		"QUEUE_CONNECTION": {IfValue: "redis", ReplaceWith: "sync"},
	}
	for key, want := range tests {
		got, ok := rules[key]
		if !ok {
			t.Errorf("missing rule for %s", key)
			continue
		}
		if got.IfValue != want.IfValue {
			t.Errorf("%s.IfValue = %q, want %q", key, got.IfValue, want.IfValue)
		}
		if got.ReplaceWith != want.ReplaceWith {
			t.Errorf("%s.ReplaceWith = %q, want %q", key, got.ReplaceWith, want.ReplaceWith)
		}
	}
}

func TestFallbackMapping_S3(t *testing.T) {
	rules := FallbackMapping("s3")
	if len(rules) != 1 {
		t.Fatalf("expected 1 rule, got %d", len(rules))
	}
	r := rules["FILESYSTEM_DISK"]
	if r.IfValue != "s3" || r.ReplaceWith != "local" {
		t.Errorf("FILESYSTEM_DISK rule = %+v", r)
	}
}

func TestFallbackMapping_Mail(t *testing.T) {
	rules := FallbackMapping("mail")
	if len(rules) != 1 {
		t.Fatalf("expected 1 rule, got %d", len(rules))
	}
	r := rules["MAIL_MAILER"]
	if r.IfValue != "smtp" || r.ReplaceWith != "log" {
		t.Errorf("MAIL_MAILER rule = %+v", r)
	}
}

func TestFallbackMapping_Unknown(t *testing.T) {
	rules := FallbackMapping("postgres")
	if rules != nil {
		t.Errorf("expected nil for unknown service, got %v", rules)
	}
}

func TestApplyFallbacks_ReplacesMatchingValues(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	os.WriteFile(envPath, []byte("CACHE_STORE=redis\nSESSION_DRIVER=redis\nQUEUE_CONNECTION=redis\n"), 0644)

	if err := ApplyFallbacks(envPath, "redis"); err != nil {
		t.Fatalf("ApplyFallbacks: %v", err)
	}

	env, err := projectenv.ReadDotEnv(envPath)
	if err != nil {
		t.Fatalf("ReadDotEnv: %v", err)
	}
	if env["CACHE_STORE"] != "file" {
		t.Errorf("CACHE_STORE = %q, want file", env["CACHE_STORE"])
	}
	if env["SESSION_DRIVER"] != "file" {
		t.Errorf("SESSION_DRIVER = %q, want file", env["SESSION_DRIVER"])
	}
	if env["QUEUE_CONNECTION"] != "sync" {
		t.Errorf("QUEUE_CONNECTION = %q, want sync", env["QUEUE_CONNECTION"])
	}
}

func TestApplyFallbacks_SkipsNonMatchingValues(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	os.WriteFile(envPath, []byte("CACHE_STORE=array\nSESSION_DRIVER=database\nQUEUE_CONNECTION=sqs\n"), 0644)

	if err := ApplyFallbacks(envPath, "redis"); err != nil {
		t.Fatalf("ApplyFallbacks: %v", err)
	}

	env, err := projectenv.ReadDotEnv(envPath)
	if err != nil {
		t.Fatalf("ReadDotEnv: %v", err)
	}
	if env["CACHE_STORE"] != "array" {
		t.Errorf("CACHE_STORE = %q, want array", env["CACHE_STORE"])
	}
	if env["SESSION_DRIVER"] != "database" {
		t.Errorf("SESSION_DRIVER = %q, want database", env["SESSION_DRIVER"])
	}
	if env["QUEUE_CONNECTION"] != "sqs" {
		t.Errorf("QUEUE_CONNECTION = %q, want sqs", env["QUEUE_CONNECTION"])
	}
}

func TestApplyFallbacks_S3(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	os.WriteFile(envPath, []byte("FILESYSTEM_DISK=s3\n"), 0644)

	if err := ApplyFallbacks(envPath, "s3"); err != nil {
		t.Fatalf("ApplyFallbacks: %v", err)
	}

	env, _ := projectenv.ReadDotEnv(envPath)
	if env["FILESYSTEM_DISK"] != "local" {
		t.Errorf("FILESYSTEM_DISK = %q, want local", env["FILESYSTEM_DISK"])
	}
}

func TestApplyFallbacks_Mail(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	os.WriteFile(envPath, []byte("MAIL_MAILER=smtp\n"), 0644)

	if err := ApplyFallbacks(envPath, "mail"); err != nil {
		t.Fatalf("ApplyFallbacks: %v", err)
	}

	env, _ := projectenv.ReadDotEnv(envPath)
	if env["MAIL_MAILER"] != "log" {
		t.Errorf("MAIL_MAILER = %q, want log", env["MAIL_MAILER"])
	}
}

func TestApplyFallbacks_NoRulesForService(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	os.WriteFile(envPath, []byte("DB_CONNECTION=mysql\n"), 0644)

	if err := ApplyFallbacks(envPath, "mysql"); err != nil {
		t.Fatalf("ApplyFallbacks: %v", err)
	}

	env, _ := projectenv.ReadDotEnv(envPath)
	if env["DB_CONNECTION"] != "mysql" {
		t.Errorf("DB_CONNECTION = %q, want mysql (unchanged)", env["DB_CONNECTION"])
	}
}

func TestUpdateProjectEnvForBinaryService(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("APP_NAME=test\n"), 0644); err != nil {
		t.Fatal(err)
	}

	svc, ok := services.LookupBinary("s3")
	if !ok {
		t.Fatal("s3 binary service not registered")
	}
	bound := &registry.ProjectServices{S3: true}

	if err := UpdateProjectEnvForBinaryService(dir, "my-app", "s3", svc, bound); err != nil {
		t.Fatalf("UpdateProjectEnvForBinaryService: %v", err)
	}

	env, _ := projectenv.ReadDotEnv(envPath)
	// Connection vars from RustFS.EnvVars
	if env["AWS_BUCKET"] != "my-app" {
		t.Errorf("AWS_BUCKET = %q, want my-app", env["AWS_BUCKET"])
	}
	if env["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Errorf("AWS_ENDPOINT = %q, want http://127.0.0.1:9000", env["AWS_ENDPOINT"])
	}
	// Smart var from SmartEnvVars — only added when project is bound to s3.
	if env["FILESYSTEM_DISK"] != "s3" {
		t.Errorf("FILESYSTEM_DISK = %q, want s3", env["FILESYSTEM_DISK"])
	}
}

func TestUpdateProjectEnvForBinaryService_NoEnvFile(t *testing.T) {
	dir := t.TempDir()
	svc, ok := services.LookupBinary("s3")
	if !ok {
		t.Fatal("s3 binary service not registered")
	}
	bound := &registry.ProjectServices{S3: true}

	if err := UpdateProjectEnvForBinaryService(dir, "my-app", "s3", svc, bound); err != nil {
		t.Fatalf("should not error for missing .env: %v", err)
	}
}

func TestUpdateProjectEnvForPostgres(t *testing.T) {
	tmp := t.TempDir()
	envPath := filepath.Join(tmp, ".env")
	if err := os.WriteFile(envPath, []byte("# initial\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	bound := &registry.ProjectServices{Postgres: "17"}
	if err := UpdateProjectEnvForPostgres(tmp, "my_app", "17", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForPostgres: %v", err)
	}
	data, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatal(err)
	}
	body := string(data)
	for _, want := range []string{"DB_CONNECTION=pgsql", "DB_PORT=54017", "DB_DATABASE=my_app"} {
		if !strings.Contains(body, want) {
			t.Errorf("missing %q in .env:\n%s", want, body)
		}
	}
}

func TestUpdateProjectEnvForMysql(t *testing.T) {
	tmp := t.TempDir()
	envPath := filepath.Join(tmp, ".env")
	if err := os.WriteFile(envPath, []byte("# initial\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	bound := &registry.ProjectServices{MySQL: "8.4"}
	if err := UpdateProjectEnvForMysql(tmp, "my_app", "8.4", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForMysql: %v", err)
	}
	data, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatal(err)
	}
	body := string(data)
	for _, want := range []string{"DB_CONNECTION=mysql", "DB_PORT=33084", "DB_DATABASE=my_app", "DB_USERNAME=root"} {
		if !strings.Contains(body, want) {
			t.Errorf("missing %q in .env:\n%s", want, body)
		}
	}
}

func TestUpdateProjectEnvForMysql_NoEnvFile(t *testing.T) {
	tmp := t.TempDir()
	bound := &registry.ProjectServices{MySQL: "8.4"}
	// No .env on disk. Must be a no-op without error (matches the postgres
	// and docker variants — pv never creates .env from nothing).
	if err := UpdateProjectEnvForMysql(tmp, "my_app", "8.4", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForMysql with no .env: %v", err)
	}
}

func TestUpdateProjectEnvForRedis(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("APP_NAME=test\nREDIS_HOST=stale\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	bound := &registry.ProjectServices{Redis: true}
	if err := UpdateProjectEnvForRedis(dir, "test", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForRedis: %v", err)
	}

	got, err := os.ReadFile(envPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"REDIS_HOST=127.0.0.1",
		"REDIS_PORT=6379",
		"REDIS_PASSWORD=null",
		"CACHE_STORE=redis",
		"SESSION_DRIVER=redis",
		"QUEUE_CONNECTION=redis",
	} {
		if !strings.Contains(string(got), want) {
			t.Errorf(".env missing %q; got: %s", want, string(got))
		}
	}
}

func TestUpdateProjectEnvForRedis_NoEnvFile(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	dir := t.TempDir()
	bound := &registry.ProjectServices{Redis: true}
	// No .env file in dir — helper must no-op without error.
	if err := UpdateProjectEnvForRedis(dir, "test", bound); err != nil {
		t.Fatalf("UpdateProjectEnvForRedis on missing .env: %v", err)
	}
}
