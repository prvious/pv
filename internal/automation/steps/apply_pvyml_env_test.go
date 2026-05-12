package steps

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
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
