package laravel

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
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

// --- CopyEnvStep tests ---

func TestCopyEnvStep_ShouldRun_FalseWithoutEnvExample(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	step := &CopyEnvStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when .env.example is missing")
	}
}

func TestCopyEnvStep_ShouldRun_TrueWhenEnvExampleExists(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env.example"), []byte("APP_KEY=\n"), 0644)
	ctx := testCtx(dir)
	step := &CopyEnvStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true when .env.example exists and .env is missing")
	}
}

func TestCopyEnvStep_ShouldRun_FalseWhenEnvExists(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env.example"), []byte("APP_KEY=\n"), 0644)
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=\n"), 0644)
	ctx := testCtx(dir)
	step := &CopyEnvStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when .env already exists")
	}
}

func TestCopyEnvStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env.example"), []byte("APP_KEY=\n"), 0644)
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &CopyEnvStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

func TestCopyEnvStep_Run(t *testing.T) {
	dir := t.TempDir()
	content := "APP_NAME=TestApp\nAPP_KEY=\nDB_HOST=localhost\n"
	os.WriteFile(filepath.Join(dir, ".env.example"), []byte(content), 0644)

	ctx := testCtx(dir)
	step := &CopyEnvStep{}

	result, err := step.Run(ctx)
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if result != "copied .env.example → .env" {
		t.Errorf("Run() result = %q", result)
	}

	// .env should exist with same content
	data, err := os.ReadFile(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	if string(data) != content {
		t.Errorf(".env content = %q, want %q", string(data), content)
	}

	// ctx.Env should be populated
	if ctx.Env["APP_NAME"] != "TestApp" {
		t.Errorf("ctx.Env[APP_NAME] = %q, want %q", ctx.Env["APP_NAME"], "TestApp")
	}
	if ctx.Env["DB_HOST"] != "localhost" {
		t.Errorf("ctx.Env[DB_HOST] = %q, want %q", ctx.Env["DB_HOST"], "localhost")
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
	env, err := services.ReadDotEnv(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	if env["APP_URL"] != "https://test-project.test" {
		t.Errorf("APP_URL = %q, want %q", env["APP_URL"], "https://test-project.test")
	}
}

// --- GenerateKeyStep tests ---

func TestGenerateKeyStep_ShouldRun_FalseWithoutEnvFile(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	step := &GenerateKeyStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when .env is missing")
	}
}

func TestGenerateKeyStep_ShouldRun_TrueWhenKeyEmpty(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=\n"), 0644)
	ctx := testCtx(dir)
	step := &GenerateKeyStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true when APP_KEY is empty")
	}
}

func TestGenerateKeyStep_ShouldRun_FalseWhenKeySet(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=base64:abc123\n"), 0644)
	ctx := testCtx(dir)
	step := &GenerateKeyStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when APP_KEY is already set")
	}
}

func TestGenerateKeyStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=\n"), 0644)
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &GenerateKeyStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

// --- InstallOctaneStep tests ---

func TestInstallOctaneStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &InstallOctaneStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

func TestInstallOctaneStep_ShouldRun_FalseWithoutOctanePackage(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte(`{"require":{"laravel/framework":"^11.0"}}`), 0644)
	ctx := testCtx(dir)
	step := &InstallOctaneStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false without octane in composer.json")
	}
}

func TestInstallOctaneStep_ShouldRun_TrueWhenOctanePackageNoWorker(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte(`{"require":{"laravel/octane":"^2.0"}}`), 0644)
	ctx := testCtx(dir)
	step := &InstallOctaneStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true when octane is in composer.json but worker is missing")
	}
}

func TestInstallOctaneStep_ShouldRun_FalseWhenWorkerExists(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte(`{"require":{"laravel/octane":"^2.0"}}`), 0644)
	os.MkdirAll(filepath.Join(dir, "public"), 0755)
	os.WriteFile(filepath.Join(dir, "public", "frankenphp-worker.php"), []byte("<?php"), 0644)
	ctx := testCtx(dir)
	step := &InstallOctaneStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when worker already exists")
	}
}

// --- ComposerInstallStep tests ---

func TestComposerInstallStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &ComposerInstallStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

func TestComposerInstallStep_ShouldRun_FalseWithoutComposerJSON(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	step := &ComposerInstallStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false without composer.json")
	}
}

func TestComposerInstallStep_ShouldRun_TrueWhenVendorMissing(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte("{}"), 0644)
	ctx := testCtx(dir)
	step := &ComposerInstallStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true when vendor/ is missing")
	}
}

func TestComposerInstallStep_ShouldRun_FalseWhenVendorExists(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte("{}"), 0644)
	os.MkdirAll(filepath.Join(dir, "vendor"), 0755)
	ctx := testCtx(dir)
	step := &ComposerInstallStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when vendor/ exists")
	}
}

// --- DetectServicesStep tests ---

func TestDetectServicesStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &DetectServicesStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

func TestDetectServicesStep_ShouldRun_FalseWithNoRegistry(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	step := &DetectServicesStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false with nil registry")
	}
}

func TestDetectServicesStep_ShouldRun_FalseNoServicesbound(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-project", Path: dir, Services: &registry.ProjectServices{}},
		},
	}
	ctx.Registry = reg
	step := &DetectServicesStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when no services are bound")
	}
}

func TestDetectServicesStep_ShouldRun_TrueWithRedis(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-project", Path: dir, Services: &registry.ProjectServices{Redis: true}},
		},
	}
	ctx.Registry = reg
	step := &DetectServicesStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true when Redis is bound")
	}
}

func TestDetectServicesStep_Run(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_NAME=Test\n"), 0644)

	ctx := testCtx(dir)
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-project", Path: dir, Services: &registry.ProjectServices{Redis: true, Mail: true}},
		},
	}
	ctx.Registry = reg
	step := &DetectServicesStep{}

	result, err := step.Run(ctx)
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if !strings.Contains(result, "4 service env vars") {
		t.Errorf("Run() result = %q, expected mention of 4 vars", result)
	}

	// Check .env was updated
	env, err := services.ReadDotEnv(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatalf("failed to read .env: %v", err)
	}
	if env["CACHE_STORE"] != "redis" {
		t.Errorf("CACHE_STORE = %q, want %q", env["CACHE_STORE"], "redis")
	}
	if env["MAIL_MAILER"] != "smtp" {
		t.Errorf("MAIL_MAILER = %q, want %q", env["MAIL_MAILER"], "smtp")
	}
}

// --- CreateDatabaseStep tests ---

func TestCreateDatabaseStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &CreateDatabaseStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

func TestCreateDatabaseStep_ShouldRun_FalseWithoutDBService(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-project", Path: dir, Services: &registry.ProjectServices{Redis: true}},
		},
	}
	ctx.Registry = reg
	step := &CreateDatabaseStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false without a database service")
	}
}

func TestCreateDatabaseStep_ShouldRun_TrueWithMySQL(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-project", Path: dir, Services: &registry.ProjectServices{MySQL: "8.0"}},
		},
	}
	ctx.Registry = reg
	step := &CreateDatabaseStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true with MySQL bound")
	}
}

func TestCreateDatabaseStep_Run(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	reg := &registry.Registry{
		Projects: []registry.Project{
			{Name: "test-project", Path: dir, Services: &registry.ProjectServices{MySQL: "8.0"}},
		},
	}
	ctx.Registry = reg
	step := &CreateDatabaseStep{}

	result, err := step.Run(ctx)
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	// Should return sanitized project name as db name
	if result != "test_project" {
		t.Errorf("Run() result = %q, want %q", result, "test_project")
	}
	if !ctx.DBCreated {
		t.Error("ctx.DBCreated should be true after Run")
	}
	// Verify database was recorded in registry (mutation persists via index access).
	proj := reg.Projects[0]
	if len(proj.Databases) != 1 || proj.Databases[0] != "test_project" {
		t.Errorf("project databases = %v, want [test_project]", proj.Databases)
	}
}

// --- RunMigrationsStep tests ---

func TestRunMigrationsStep_ShouldRun_FalseForNonLaravel(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.ProjectType = "php"
	step := &RunMigrationsStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false for non-Laravel projects")
	}
}

func TestRunMigrationsStep_ShouldRun_FalseWhenDBNotCreated(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	step := &RunMigrationsStep{}

	if step.ShouldRun(ctx) {
		t.Error("ShouldRun should be false when DBCreated is false")
	}
}

func TestRunMigrationsStep_ShouldRun_TrueWhenDBCreated(t *testing.T) {
	dir := t.TempDir()
	ctx := testCtx(dir)
	ctx.DBCreated = true
	step := &RunMigrationsStep{}

	if !step.ShouldRun(ctx) {
		t.Error("ShouldRun should be true when DBCreated is true")
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
