package service

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

func TestApplyFallbacksToLinkedProjects_Integration(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	os.WriteFile(envPath, []byte("CACHE_STORE=redis\nSESSION_DRIVER=redis\n"), 0644)

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"redis": {Image: "redis:latest", Port: 6379},
		},
		Projects: []registry.Project{
			{Name: "test-app", Path: projectDir, Type: "laravel",
				Services: &registry.ProjectServices{Redis: true}},
		},
	}

	origConfirm := automation.ConfirmFunc
	automation.ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { automation.ConfirmFunc = origConfirm }()

	applyFallbacksToLinkedProjects(reg, "redis")

	env, _ := services.ReadDotEnv(envPath)
	if env["CACHE_STORE"] != "file" {
		t.Errorf("CACHE_STORE = %q, want file", env["CACHE_STORE"])
	}
	if env["SESSION_DRIVER"] != "file" {
		t.Errorf("SESSION_DRIVER = %q, want file", env["SESSION_DRIVER"])
	}
}

func TestApplyFallbacksToLinkedProjects_Mail(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projectDir := t.TempDir()
	envPath := filepath.Join(projectDir, ".env")
	os.WriteFile(envPath, []byte("MAIL_MAILER=smtp\n"), 0644)

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Kind: "binary", Port: 1025},
		},
		Projects: []registry.Project{
			{Name: "test-app", Path: projectDir, Type: "laravel",
				Services: &registry.ProjectServices{Mail: true}},
		},
	}

	origConfirm := automation.ConfirmFunc
	automation.ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { automation.ConfirmFunc = origConfirm }()

	applyFallbacksToLinkedProjects(reg, "mail")

	env, _ := services.ReadDotEnv(envPath)
	if env["MAIL_MAILER"] != "log" {
		t.Errorf("MAIL_MAILER = %q, want log", env["MAIL_MAILER"])
	}
}

// TestStopAllFallbackLoop_SkipsBinaryServices simulates the stop-all fallback
// loop from stop.go:64-68 and verifies that binary services are skipped while
// Docker services still get fallbacks applied.
func TestStopAllFallbackLoop_SkipsBinaryServices(t *testing.T) {
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

	// Simulate the stop-all fallback loop (same logic as stop.go:64-68).
	for key, inst := range reg.ListServices() {
		if inst.Kind == "binary" {
			continue
		}
		svcName, _ := services.ParseServiceKey(key)
		applyFallbacksToLinkedProjects(reg, svcName)
	}

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
