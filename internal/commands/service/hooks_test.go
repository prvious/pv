package service

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

func TestExtractServiceName(t *testing.T) {
	tests := []struct{ key, want string }{
		{"mysql:8.0.32", "mysql"},
		{"redis", "redis"},
		{"postgres:16", "postgres"},
	}
	for _, tt := range tests {
		if got := extractServiceName(tt.key); got != tt.want {
			t.Errorf("extractServiceName(%q) = %q, want %q", tt.key, got, tt.want)
		}
	}
}

func TestExtractVersion(t *testing.T) {
	tests := []struct{ key, want string }{
		{"mysql:8.0.32", "8.0.32"},
		{"redis", "latest"},
		{"postgres:16", "16"},
	}
	for _, tt := range tests {
		if got := extractVersion(tt.key); got != tt.want {
			t.Errorf("extractVersion(%q) = %q, want %q", tt.key, got, tt.want)
		}
	}
}

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
	automation.ConfirmFunc = func(label string) bool { return true }
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
