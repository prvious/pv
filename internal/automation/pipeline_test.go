package automation

import (
	"testing"

	"github.com/prvious/pv/internal/config"
)

// stubStep implements Step for testing.
type stubStep struct {
	label     string
	gate      string
	shouldRun bool
	result    string
	err       error
	ran       bool
}

func (s *stubStep) Label() string             { return s.label }
func (s *stubStep) Gate() string              { return s.gate }
func (s *stubStep) ShouldRun(_ *Context) bool { return s.shouldRun }
func (s *stubStep) Run(_ *Context) (string, error) {
	s.ran = true
	return s.result, s.err
}

func defaultCtx() *Context {
	s := config.DefaultSettings()
	return &Context{
		ProjectPath: "/tmp/test-project",
		ProjectName: "test-project",
		Settings:    s,
		Env:         make(map[string]string),
	}
}

func TestRunPipeline_SkipsWhenShouldRunFalse(t *testing.T) {
	step := &stubStep{
		label:     "skip me",
		gate:      "composer_install",
		shouldRun: false,
		result:    "done",
	}

	ctx := defaultCtx()
	if err := RunPipeline([]Step{step}, ctx); err != nil {
		t.Fatalf("RunPipeline() error = %v", err)
	}
	if step.ran {
		t.Error("step should not have run when ShouldRun returns false")
	}
}

func TestRunPipeline_SkipsWhenGateOff(t *testing.T) {
	step := &stubStep{
		label:     "composer install",
		gate:      "composer_install",
		shouldRun: true,
		result:    "installed",
	}

	ctx := defaultCtx()
	ctx.Settings.Automation.ComposerInstall = config.AutoOff

	if err := RunPipeline([]Step{step}, ctx); err != nil {
		t.Fatalf("RunPipeline() error = %v", err)
	}
	if step.ran {
		t.Error("step should not have run when gate is AutoOff")
	}
}

func TestRunPipeline_RunsWhenGateOn(t *testing.T) {
	step := &stubStep{
		label:     "composer install",
		gate:      "composer_install",
		shouldRun: true,
		result:    "installed",
	}

	ctx := defaultCtx()
	ctx.Settings.Automation.ComposerInstall = config.AutoOn

	if err := RunPipeline([]Step{step}, ctx); err != nil {
		t.Fatalf("RunPipeline() error = %v", err)
	}
	if !step.ran {
		t.Error("step should have run when gate is AutoOn")
	}
}

func TestRunPipeline_AskTreatedAsOffWhenNonInteractive(t *testing.T) {
	step := &stubStep{
		label:     "run migrations",
		gate:      "run_migrations",
		shouldRun: true,
		result:    "migrated",
	}

	ctx := defaultCtx()
	ctx.Settings.Automation.RunMigrations = config.AutoAsk

	// Force non-interactive
	origIsInteractive := isInteractiveFunc
	isInteractiveFunc = func() bool { return false }
	defer func() { isInteractiveFunc = origIsInteractive }()

	if err := RunPipeline([]Step{step}, ctx); err != nil {
		t.Fatalf("RunPipeline() error = %v", err)
	}
	if step.ran {
		t.Error("step should not have run when gate is AutoAsk and non-interactive")
	}
}

func TestRunPipeline_AskRunsWhenConfirmed(t *testing.T) {
	step := &stubStep{
		label:     "run migrations",
		gate:      "run_migrations",
		shouldRun: true,
		result:    "migrated",
	}

	ctx := defaultCtx()
	ctx.Settings.Automation.RunMigrations = config.AutoAsk

	// Force interactive + confirm yes
	origIsInteractive := isInteractiveFunc
	isInteractiveFunc = func() bool { return true }
	defer func() { isInteractiveFunc = origIsInteractive }()

	origConfirm := ConfirmFunc
	ConfirmFunc = func(label string) (bool, error) { return true, nil }
	defer func() { ConfirmFunc = origConfirm }()

	if err := RunPipeline([]Step{step}, ctx); err != nil {
		t.Fatalf("RunPipeline() error = %v", err)
	}
	if !step.ran {
		t.Error("step should have run when gate is AutoAsk and user confirms")
	}
}

func TestRunPipeline_AskSkipsWhenDenied(t *testing.T) {
	step := &stubStep{
		label:     "run migrations",
		gate:      "run_migrations",
		shouldRun: true,
		result:    "migrated",
	}

	ctx := defaultCtx()
	ctx.Settings.Automation.RunMigrations = config.AutoAsk

	// Force interactive + confirm no
	origIsInteractive := isInteractiveFunc
	isInteractiveFunc = func() bool { return true }
	defer func() { isInteractiveFunc = origIsInteractive }()

	origConfirm := ConfirmFunc
	ConfirmFunc = func(label string) (bool, error) { return false, nil }
	defer func() { ConfirmFunc = origConfirm }()

	if err := RunPipeline([]Step{step}, ctx); err != nil {
		t.Fatalf("RunPipeline() error = %v", err)
	}
	if step.ran {
		t.Error("step should not have run when gate is AutoAsk and user denies")
	}
}

func TestLookupGate_InstallPHPVersion(t *testing.T) {
	a := config.DefaultAutomation()
	mode := LookupGate(&a, "install_php_version")
	if mode != config.AutoOn {
		t.Errorf("LookupGate(install_php_version) = %q, want %q", mode, config.AutoOn)
	}

	a.InstallPHPVersion = config.AutoAsk
	mode = LookupGate(&a, "install_php_version")
	if mode != config.AutoAsk {
		t.Errorf("LookupGate(install_php_version) = %q, want %q", mode, config.AutoAsk)
	}
}

func TestLookupGate(t *testing.T) {
	a := config.DefaultAutomation()

	tests := []struct {
		gate string
		want config.AutoMode
	}{
		{"install_php_version", a.InstallPHPVersion},
		{"composer_install", a.ComposerInstall},
		{"copy_env", a.CopyEnv},
		{"generate_key", a.GenerateKey},
		{"set_app_url", a.SetAppURL},
		{"install_octane", a.InstallOctane},
		{"create_database", a.CreateDatabase},
		{"run_migrations", a.RunMigrations},
		{"update_env_on_service", a.ServiceEnvUpdate},
		{"service_fallback", a.ServiceFallback},
		{"unknown_gate", config.AutoAsk},
	}

	for _, tt := range tests {
		t.Run(tt.gate, func(t *testing.T) {
			got := LookupGate(&a, tt.gate)
			if got != tt.want {
				t.Errorf("LookupGate(%q) = %q, want %q", tt.gate, got, tt.want)
			}
		})
	}
}
