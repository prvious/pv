package laravel

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/projectenv"
)

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
