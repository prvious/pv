package laravel

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
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
	env, err := projectenv.ReadDotEnv(filepath.Join(dir, ".env"))
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
	t.Setenv("HOME", t.TempDir())
	// Stage a fake `mysql` client so CreateDatabase can shell out without
	// the real binary present.
	binDir := config.MysqlBinDir("8.0")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	stub := "#!/bin/sh\nexit 0\n"
	if err := os.WriteFile(filepath.Join(binDir, "mysql"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}

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

func TestDetectServicesStep_WritesMysqlEnvVars(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("APP_NAME=demo\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{
			{Name: "demo", Path: dir, Type: "laravel",
				Services: &registry.ProjectServices{MySQL: "8.4"}},
		},
	}
	ctx := &automation.Context{
		ProjectName: "demo",
		ProjectPath: dir,
		ProjectType: "laravel",
		Registry:    reg,
	}
	step := &DetectServicesStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	body, _ := os.ReadFile(envPath)
	for _, want := range []string{"DB_CONNECTION=mysql", "DB_PORT=33084", "DB_DATABASE=demo"} {
		if !strings.Contains(string(body), want) {
			t.Errorf("missing %q in .env:\n%s", want, string(body))
		}
	}
}

// laravelCtxWithBoundService returns an automation.Context that
// satisfies every condition of DetectServicesStep.ShouldRun EXCEPT
// the pv.yml short-circuit — useful for verifying the short-circuit
// is the only thing toggling ShouldRun in these tests.
func laravelCtxWithBoundService(name string) *automation.Context {
	return &automation.Context{
		ProjectName: name,
		ProjectType: "laravel",
		Registry: &registry.Registry{
			Projects: []registry.Project{{
				Name:     name,
				Type:     "laravel",
				Services: &registry.ProjectServices{Redis: true},
			}},
		},
	}
}

func TestLaravelDetectServices_SkipsWhenPvYmlHasEnv(t *testing.T) {
	ctx := laravelCtxWithBoundService("test-project")
	ctx.ProjectConfig = &config.ProjectConfig{
		Env: map[string]string{"APP_URL": "{{ .site_url }}"},
	}
	step := &DetectServicesStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when pv.yml has env")
	}
}

func TestLaravelDetectServices_RunsWhenPvYmlEmpty(t *testing.T) {
	ctx := laravelCtxWithBoundService("test-project")
	ctx.ProjectConfig = &config.ProjectConfig{PHP: "8.4"}
	step := &DetectServicesStep{}
	if !step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want true when pv.yml has no env")
	}
}

// --- HasSetup() short-circuit tests ---
//
// Each test below sets up the minimum filesystem/registry state so that
// ShouldRun returns true with no Setup declared (baseline assertion),
// and verifies it returns false when Setup is declared (gated assertion).
// Both halves in one test make the test self-validating: a regression
// that removes the HasSetup short-circuit would fail the gated half.

func TestCopyEnvStep_SkipsWhenSetupDeclared(t *testing.T) {
	dir := t.TempDir()
	// .env.example exists, no .env → ShouldRun would normally return true.
	if err := os.WriteFile(filepath.Join(dir, ".env.example"), []byte("APP_NAME=Test\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	step := &CopyEnvStep{}

	// Sanity: without setup, the step WOULD run. If this fails, the test
	// setup is broken and the assertion below would pass for the wrong
	// reason.
	baseline := &automation.Context{
		ProjectType: "laravel",
		ProjectPath: dir,
	}
	if !step.ShouldRun(baseline) {
		t.Fatalf("test invariant: CopyEnvStep should run when .env.example exists without setup")
	}

	// With setup, the step skips.
	gated := &automation.Context{
		ProjectType:   "laravel",
		ProjectPath:   dir,
		ProjectConfig: &config.ProjectConfig{Setup: []string{"cp .env.example .env"}},
	}
	if step.ShouldRun(gated) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}

func TestGenerateKeyStep_SkipsWhenSetupDeclared(t *testing.T) {
	dir := t.TempDir()
	// .env exists with empty APP_KEY → ShouldRun would normally return true.
	if err := os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	step := &GenerateKeyStep{}

	baseline := &automation.Context{
		ProjectType: "laravel",
		ProjectPath: dir,
	}
	if !step.ShouldRun(baseline) {
		t.Fatalf("test invariant: GenerateKeyStep should run when .env has empty APP_KEY")
	}

	gated := &automation.Context{
		ProjectType:   "laravel",
		ProjectPath:   dir,
		ProjectConfig: &config.ProjectConfig{Setup: []string{"php artisan key:generate"}},
	}
	if step.ShouldRun(gated) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}

func TestInstallOctaneStep_SkipsWhenSetupDeclared(t *testing.T) {
	dir := t.TempDir()
	// composer.json declares laravel/octane → HasOctanePackage true.
	// No public/frankenphp-worker.php → !HasOctaneWorker.
	composer := `{"require": {"laravel/octane": "*"}}`
	if err := os.WriteFile(filepath.Join(dir, "composer.json"), []byte(composer), 0o644); err != nil {
		t.Fatal(err)
	}

	step := &InstallOctaneStep{}

	baseline := &automation.Context{
		ProjectType: "laravel",
		ProjectPath: dir,
	}
	if !step.ShouldRun(baseline) {
		t.Fatalf("test invariant: InstallOctaneStep should run when octane is in composer.json without worker")
	}

	gated := &automation.Context{
		ProjectType:   "laravel",
		ProjectPath:   dir,
		ProjectConfig: &config.ProjectConfig{Setup: []string{"composer require laravel/octane"}},
	}
	if step.ShouldRun(gated) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}

func TestComposerInstallStep_SkipsWhenSetupDeclared(t *testing.T) {
	dir := t.TempDir()
	// composer.json exists, no vendor/ → ShouldRun would normally return true.
	if err := os.WriteFile(filepath.Join(dir, "composer.json"), []byte(`{}`), 0o644); err != nil {
		t.Fatal(err)
	}

	step := &ComposerInstallStep{}

	baseline := &automation.Context{
		ProjectType: "laravel",
		ProjectPath: dir,
	}
	if !step.ShouldRun(baseline) {
		t.Fatalf("test invariant: ComposerInstallStep should run when composer.json exists without vendor/")
	}

	gated := &automation.Context{
		ProjectType:   "laravel",
		ProjectPath:   dir,
		ProjectConfig: &config.ProjectConfig{Setup: []string{"composer install"}},
	}
	if step.ShouldRun(gated) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}

func TestCreateDatabaseStep_SkipsWhenSetupDeclared(t *testing.T) {
	dir := t.TempDir()
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{},
		Projects: []registry.Project{{
			Name: "p", Path: dir, Type: "laravel",
			Services: &registry.ProjectServices{Postgres: "18"},
		}},
	}

	step := &CreateDatabaseStep{}

	baseline := &automation.Context{
		ProjectName: "p",
		ProjectType: "laravel",
		ProjectPath: dir,
		Registry:    reg,
	}
	if !step.ShouldRun(baseline) {
		t.Fatalf("test invariant: CreateDatabaseStep should run when a DB service is bound")
	}

	gated := &automation.Context{
		ProjectName:   "p",
		ProjectType:   "laravel",
		ProjectPath:   dir,
		Registry:      reg,
		ProjectConfig: &config.ProjectConfig{Setup: []string{"pv postgres:db:create my_app"}},
	}
	if step.ShouldRun(gated) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}

func TestRunMigrationsStep_SkipsWhenSetupDeclared(t *testing.T) {
	dir := t.TempDir()

	step := &RunMigrationsStep{}

	baseline := &automation.Context{
		ProjectType: "laravel",
		ProjectPath: dir,
		DBCreated:   true,
	}
	if !step.ShouldRun(baseline) {
		t.Fatalf("test invariant: RunMigrationsStep should run when DBCreated is true")
	}

	gated := &automation.Context{
		ProjectType:   "laravel",
		ProjectPath:   dir,
		DBCreated:     true,
		ProjectConfig: &config.ProjectConfig{Setup: []string{"php artisan migrate"}},
	}
	if step.ShouldRun(gated) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}
