package laravel

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/projectenv"
)

func testCtx(dir string) *automation.Context {
	s := config.DefaultSettings()
	return &automation.Context{
		ProjectPath: dir,
		ProjectName: "test-project",
		ProjectType: "laravel",
		TLD:         "test",
		Settings:    s,
		Env:         make(map[string]string),
	}
}

// --- SetAppURLStep tests ---

func TestSetAppURLStep_ShouldRun_TrueForLaravel(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=http://localhost\n"), 0644)
	ctx := testCtx(dir)
	step := &SetAppURLStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true for Laravel projects with .env")
	}
}

func TestSetAppURLStep_ShouldRun_TrueForLaravelOctane(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=http://localhost\n"), 0644)
	ctx := testCtx(dir)
	ctx.ProjectType = "laravel-octane"
	step := &SetAppURLStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true for laravel-octane projects with .env")
	}
}

func TestSetAppURLStep_ShouldRun_FalseWhenNoEnv(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	step := &SetAppURLStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when .env does not exist")
	}
}

func TestSetAppURLStep_ShouldRun_FalseForPHP(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &SetAppURLStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for php projects")
	}
}

func TestSetAppURLStep_Run(t *testing.T) {
	dir := t.TempDir()
	// Create .env for MergeDotEnv to work on
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=http://localhost\n"), 0644)

	ctx := testCtx(dir)
	step := &SetAppURLStep{}

	result, err := step.Run(ctx)
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if result != "https://test-project.test" {
		t.Errorf("Run() result = %q, want %q", result, "https://test-project.test")
	}

	// Verify .env was updated
	env, err := projectenv.ReadDotEnv(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	if env["APP_URL"] != "https://test-project.test" {
		t.Errorf("APP_URL = %q, want %q", env["APP_URL"], "https://test-project.test")
	}
}

// --- SetViteTLSStep tests ---

func TestSetViteTLSStep_ShouldRun_TrueForLaravel(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=http://localhost"), 0644)

	step := &SetViteTLSStep{}
	ctx := &automation.Context{ProjectType: "laravel", ProjectPath: dir}
	if !step.ShouldRun(ctx) {
		t.Error("expected ShouldRun=true for laravel with .env")
	}
}

func TestSetViteTLSStep_ShouldRun_FalseWhenNoEnv(t *testing.T) {
	dir := t.TempDir()
	step := &SetViteTLSStep{}
	ctx := &automation.Context{ProjectType: "laravel", ProjectPath: dir}
	if step.ShouldRun(ctx) {
		t.Error("expected ShouldRun=false when no .env")
	}
}

func TestSetViteTLSStep_ShouldRun_FalseForPHP(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte(""), 0644)
	step := &SetViteTLSStep{}
	ctx := &automation.Context{ProjectType: "php", ProjectPath: dir}
	if step.ShouldRun(ctx) {
		t.Error("expected ShouldRun=false for php")
	}
}

func TestSetViteTLSStep_Run(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=https://myapp.test\n"), 0644)

	step := &SetViteTLSStep{}
	ctx := &automation.Context{
		ProjectPath: dir,
		ProjectName: "myapp",
		TLD:         "test",
	}

	result, err := step.Run(ctx)
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if result == "" {
		t.Error("expected non-empty result")
	}

	env, err := projectenv.ReadDotEnv(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatalf("ReadDotEnv: %v", err)
	}

	certPath := certs.CertPath("myapp.test")
	keyPath := certs.KeyPath("myapp.test")

	if env["VITE_DEV_SERVER_CERT"] != certPath {
		t.Errorf("VITE_DEV_SERVER_CERT = %q, want %q", env["VITE_DEV_SERVER_CERT"], certPath)
	}
	if env["VITE_DEV_SERVER_KEY"] != keyPath {
		t.Errorf("VITE_DEV_SERVER_KEY = %q, want %q", env["VITE_DEV_SERVER_KEY"], keyPath)
	}
}

// --- isLaravel tests ---

func TestIsLaravel(t *testing.T) {
	tests := []struct {
		input string
		want  bool
	}{
		{"laravel", true},
		{"laravel-octane", true},
		{"php", false},
		{"static", false},
		{"", false},
	}
	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			if got := isLaravel(tt.input); got != tt.want {
				t.Errorf("isLaravel(%q) = %v, want %v", tt.input, got, tt.want)
			}
		})
	}
}
