package service

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

// TestApplyStopAllFallbacks verifies the production function that stop.go
// calls in the no-args path. Docker services get fallbacks; binary
// services are skipped (they are owned by rustfs:* / mailpit:* now and
// were never stopped by service:stop).
func TestApplyStopAllFallbacks(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Two linked projects: one using redis (Docker), one using mail (binary).
	redisProjectDir := t.TempDir()
	os.WriteFile(filepath.Join(redisProjectDir, ".env"),
		[]byte("CACHE_STORE=redis\n"), 0644)

	mailProjectDir := t.TempDir()
	os.WriteFile(filepath.Join(mailProjectDir, ".env"),
		[]byte("MAIL_MAILER=smtp\n"), 0644)

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"redis": {Image: "redis:latest", Port: 6379},
			"mail":  {Kind: "binary", Port: 1025},
		},
		Projects: []registry.Project{
			{Name: "redis-app", Path: redisProjectDir, Type: "laravel",
				Services: &registry.ProjectServices{Redis: true}},
			{Name: "mail-app", Path: mailProjectDir, Type: "laravel",
				Services: &registry.ProjectServices{Mail: true}},
		},
	}

	origConfirm := automation.ConfirmFunc
	automation.ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { automation.ConfirmFunc = origConfirm }()

	// Call the production function — same code stop.go uses.
	applyStopAllFallbacks(reg)

	// Docker service (redis) should have fallback applied.
	redisEnv, _ := services.ReadDotEnv(filepath.Join(redisProjectDir, ".env"))
	if redisEnv["CACHE_STORE"] != "file" {
		t.Errorf("redis CACHE_STORE = %q, want file", redisEnv["CACHE_STORE"])
	}

	// Binary service (mail) should NOT have fallback applied.
	mailEnv, _ := services.ReadDotEnv(filepath.Join(mailProjectDir, ".env"))
	if mailEnv["MAIL_MAILER"] != "smtp" {
		t.Errorf("mail MAIL_MAILER = %q, want smtp (should NOT have been changed)",
			mailEnv["MAIL_MAILER"])
	}
}

// TestUpdateLinkedProjectsEnv_OnlyUpdatesLinkedProject verifies that when a
// service is added/started, only projects linked to that service have their
// .env updated — not unrelated projects that happen to be in the registry.
func TestUpdateLinkedProjectsEnv_OnlyUpdatesLinkedProject(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	redisProjectDir := t.TempDir()
	os.WriteFile(filepath.Join(redisProjectDir, ".env"),
		[]byte("APP_NAME=demo\n"), 0644)

	otherProjectDir := t.TempDir()
	os.WriteFile(filepath.Join(otherProjectDir, ".env"),
		[]byte("APP_NAME=other\n"), 0644)

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"redis": {Image: "redis:latest", Port: 6379},
		},
		Projects: []registry.Project{
			{Name: "redis-app", Path: redisProjectDir, Type: "laravel",
				Services: &registry.ProjectServices{Redis: true}},
			{Name: "other-app", Path: otherProjectDir, Type: "laravel",
				Services: &registry.ProjectServices{}},
		},
	}

	origConfirm := automation.ConfirmFunc
	automation.ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { automation.ConfirmFunc = origConfirm }()

	updateLinkedProjectsEnv(reg, "redis", &services.Redis{}, "latest")

	redisEnv, _ := services.ReadDotEnv(filepath.Join(redisProjectDir, ".env"))
	if redisEnv["REDIS_HOST"] != "127.0.0.1" {
		t.Errorf("redis-app REDIS_HOST = %q, want 127.0.0.1", redisEnv["REDIS_HOST"])
	}

	otherEnv, _ := services.ReadDotEnv(filepath.Join(otherProjectDir, ".env"))
	if _, ok := otherEnv["REDIS_HOST"]; ok {
		t.Errorf("other-app REDIS_HOST should be unset, got %q", otherEnv["REDIS_HOST"])
	}
}

func TestUnbindService_ClearsMailBinding(t *testing.T) {
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-app", Path: "/tmp/test",
				Services: &registry.ProjectServices{Mail: true, Redis: true}},
		},
	}

	reg.UnbindService("mail")

	project := reg.Find("test-app")
	if project.Services.Mail {
		t.Error("ProjectServices.Mail should be false after UnbindService")
	}
	// Redis should be untouched.
	if !project.Services.Redis {
		t.Error("ProjectServices.Redis should still be true")
	}
}
