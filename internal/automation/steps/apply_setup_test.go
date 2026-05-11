package steps

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
)

func TestApplySetup_RunsCommandsInOrder(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	marker := filepath.Join(projDir, "marker")

	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{
				"echo first > " + marker,
				"echo second >> " + marker,
			},
		},
	}
	step := &ApplySetupStep{}
	if !step.ShouldRun(ctx) {
		t.Fatal("ShouldRun: want true")
	}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	got := strings.TrimSpace(string(body))
	want := "first\nsecond"
	if got != want {
		t.Errorf("marker = %q, want %q", got, want)
	}
}

func TestApplySetup_FailFast(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()
	marker := filepath.Join(projDir, "marker")

	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{
				"echo first > " + marker,
				"false",
				"echo third >> " + marker,
			},
		},
	}
	step := &ApplySetupStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error after `false`, got nil")
	}
	if !strings.Contains(err.Error(), "setup[1]") {
		t.Errorf("err = %v; want it to mention setup[1]", err)
	}

	body, _ := os.ReadFile(marker)
	got := strings.TrimSpace(string(body))
	if got != "first" {
		t.Errorf("marker = %q; want only 'first' (third should not have run)", got)
	}
}

func TestApplySetup_ShouldRunFalseWithoutSetup(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &ApplySetupStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when no setup declared")
	}
}

func TestApplySetup_ShouldRunFalseWithoutConfig(t *testing.T) {
	ctx := &automation.Context{}
	step := &ApplySetupStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when ProjectConfig is nil")
	}
}

func TestApplySetup_RunsInProjectDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{"pwd > pwd-marker"},
		},
	}
	step := &ApplySetupStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	body, err := os.ReadFile(filepath.Join(projDir, "pwd-marker"))
	if err != nil {
		t.Fatal(err)
	}
	resolved, _ := filepath.EvalSymlinks(projDir)
	got := strings.TrimSpace(string(body))
	if got != projDir && got != resolved {
		t.Errorf("pwd = %q; want %q or %q", got, projDir, resolved)
	}
}
