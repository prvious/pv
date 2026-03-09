package laravel

import (
	"testing"

	"github.com/prvious/pv/internal/registry"
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
