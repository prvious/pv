package steps

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
)

func TestApplyPvYmlEnv_RendersTopLevelEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".env"), []byte("EXISTING=value\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{
				"APP_URL":  "{{ .site_url }}",
				"APP_NAME": "MyApp",
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if !step.ShouldRun(ctx) {
		t.Fatal("ShouldRun: want true")
	}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	if !strings.Contains(s, "APP_URL=https://myapp.test") {
		t.Errorf(".env missing APP_URL=https://myapp.test\n%s", s)
	}
	if !strings.Contains(s, "APP_NAME=MyApp") {
		t.Errorf(".env missing APP_NAME=MyApp\n%s", s)
	}
	if !strings.Contains(s, "EXISTING=value") {
		t.Errorf(".env clobbered existing key\n%s", s)
	}
}

func TestApplyPvYmlEnv_LabelsRenderedKeysAsManaged(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, ".env"), []byte("APP_URL=http://old.test\nCUSTOM_THING=keep-me\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{
				"APP_URL": "{{ .site_url }}",
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	want := "# pv-managed\nAPP_URL=https://myapp.test\nCUSTOM_THING=keep-me\n"
	if string(body) != want {
		t.Errorf(".env = %q, want %q", string(body), want)
	}
}

func TestApplyPvYmlEnv_RendersRedisEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	// No pre-existing .env — MergeManagedDotEnv should create it.

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Redis: &config.ServiceConfig{
				Env: map[string]string{
					"REDIS_HOST": "{{ .host }}",
					"REDIS_PORT": "{{ .port }}",
					"REDIS_URL":  "{{ .url }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	for _, want := range []string{
		"REDIS_HOST=127.0.0.1",
		"REDIS_PORT=7160",
		"REDIS_URL=redis://127.0.0.1:7160",
	} {
		if !strings.Contains(s, want) {
			t.Errorf(".env missing %q\n%s", want, s)
		}
	}
}

func TestApplyPvYmlEnv_ResolvesDefaultPostgresVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	writeExecutable(t, filepath.Join(config.PostgresBinDir(postgres.DefaultVersion()), "pg_config"), "#!/bin/sh\necho 'PostgreSQL 18.2'\n")

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{
				Env: map[string]string{
					"PG_VERSION": "{{ .version }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), "PG_VERSION=18.2") {
		t.Errorf(".env missing resolved postgres version\n%s", body)
	}
}

func TestApplyPvYmlEnv_ResolvesDefaultMysqlVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	writeExecutable(t, filepath.Join(config.MysqlBinDir(mysql.DefaultVersion()), "mysqld"), "#!/bin/sh\necho 'mysqld  Ver 8.4.9 for macos15 on arm64 (MySQL Community Server - GPL)'\n")

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Mysql: &config.ServiceConfig{
				Env: map[string]string{
					"MYSQL_VERSION": "{{ .version }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), "MYSQL_VERSION=8.4.9") {
		t.Errorf(".env missing resolved mysql version\n%s", body)
	}
}

func TestApplyPvYmlEnv_RejectsUnsupportedMailpitVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: t.TempDir(),
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Mailpit: &config.ServiceConfig{
				Version: "2",
				Env: map[string]string{
					"MAIL_HOST": "{{ .smtp_host }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error for unsupported mailpit version")
	}
	if !strings.Contains(err.Error(), "unsupported version") {
		t.Errorf("err = %v; want unsupported version", err)
	}
}

func TestApplyPvYmlEnv_RejectsUnsupportedRustfsVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: t.TempDir(),
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Rustfs: &config.ServiceConfig{
				Version: "2.0.0",
				Env: map[string]string{
					"S3_ENDPOINT": "{{ .endpoint }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error for unsupported rustfs version")
	}
	if !strings.Contains(err.Error(), "unsupported version") {
		t.Errorf("err = %v; want unsupported version", err)
	}
}

func TestApplyPvYmlEnv_RejectsUnsupportedRedisVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Redis: &config.ServiceConfig{
				Version: "7.4",
				Env: map[string]string{
					"REDIS_PORT": "{{ .port }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error for unsupported redis version")
	}
	if !strings.Contains(err.Error(), "unsupported redis version") {
		t.Errorf("err = %v; want unsupported redis version", err)
	}
}

func TestApplyPvYmlEnv_ShouldRunFalseWithoutEnv(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &ApplyPvYmlEnvStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when no env declared")
	}
}

func TestApplyPvYmlEnv_ShouldRunFalseWithoutConfig(t *testing.T) {
	ctx := &automation.Context{}
	step := &ApplyPvYmlEnvStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when ProjectConfig is nil")
	}
}

func TestApplyPvYmlEnv_ErrorsOnUnknownTemplateVar(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{
				"BAD": "{{ .nonexistent }}",
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err == nil {
		t.Fatal("Run: want error on unknown template var, got nil")
	}
}

func TestApplyPvYmlEnv_LabelsServiceEnvAsManaged(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	// No pre-existing .env — MergeManagedDotEnv should create it.

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Redis: &config.ServiceConfig{
				Env: map[string]string{
					"REDIS_HOST": "{{ .host }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, ".env"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	if !strings.Contains(s, "# pv-managed\nREDIS_HOST=127.0.0.1") {
		t.Errorf(".env missing managed marker for service key\n%s", s)
	}
}

func TestApplyPvYmlEnv_ErrorsOnDuplicateKeyAcrossScopes(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()

	ctx := &automation.Context{
		ProjectName: "myapp",
		ProjectPath: projDir,
		TLD:         "test",
		ProjectConfig: &config.ProjectConfig{
			Env: map[string]string{
				"APP_URL": "{{ .site_url }}",
			},
			Redis: &config.ServiceConfig{
				Env: map[string]string{
					"APP_URL": "{{ .host }}",
				},
			},
		},
	}
	step := &ApplyPvYmlEnvStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error on duplicate key across scopes, got nil")
	}
	if !strings.Contains(err.Error(), "duplicate env key") {
		t.Errorf("err = %v; want it to contain 'duplicate env key'", err)
	}
}

func writeExecutable(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir executable dir: %v", err)
	}
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatalf("write executable: %v", err)
	}
}
