package steps

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
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

func TestBuildSetupEnv_EmptyVersionReturnsHostEnv(t *testing.T) {
	env := buildSetupEnv("")
	// Same length as os.Environ(); contents preserved.
	if len(env) != len(os.Environ()) {
		t.Errorf("len = %d, want %d", len(env), len(os.Environ()))
	}
}

func TestBuildSetupEnv_PrependsPHPBinDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// PHPPath builds the path mechanically
	// (filepath.Join(config.PhpVersionDir(version), "php")), so we
	// don't need to actually stage the binary — the path is the
	// contract being tested.
	t.Setenv("PATH", "/usr/local/bin:/usr/bin")

	env := buildSetupEnv("8.4")

	var pathLine string
	for _, e := range env {
		if rest, ok := strings.CutPrefix(e, "PATH="); ok {
			pathLine = rest
			break
		}
	}
	if pathLine == "" {
		t.Fatal("env has no PATH entry")
	}
	want := filepath.Dir(phpenv.PHPPath("8.4"))
	if !strings.HasPrefix(pathLine, want+":") {
		t.Errorf("PATH = %q, want it to start with %q:", pathLine, want)
	}
	if !strings.Contains(pathLine, "/usr/local/bin:/usr/bin") {
		t.Errorf("PATH = %q, expected original PATH preserved", pathLine)
	}
}

func TestBuildSetupEnv_AppendsPATHWhenAbsent(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("PATH", "")
	env := buildSetupEnv("8.4")
	var pathLine string
	for _, e := range env {
		if rest, ok := strings.CutPrefix(e, "PATH="); ok {
			pathLine = rest
		}
	}
	want := filepath.Dir(phpenv.PHPPath("8.4"))
	// When original PATH is empty, the result PATH is either "<binDir>:"
	// (if PATH= was in os.Environ()) or just "<binDir>" (if it was
	// absent and we hit the append branch).
	if pathLine != want+":" && pathLine != want {
		t.Errorf("PATH = %q, want %q or %q", pathLine, want+":", want)
	}
}

func TestApplySetup_ErrorIncludesExitCodeAndStderr(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()

	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{`bash -c "echo failure-message >&2; exit 42"`},
		},
	}
	step := &ApplySetupStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error, got nil")
	}
	if !strings.Contains(err.Error(), "exited 42") {
		t.Errorf("err = %v; want it to include exit code 42", err)
	}
	if !strings.Contains(err.Error(), "failure-message") {
		t.Errorf("err = %v; want it to include stderr tail", err)
	}
}
