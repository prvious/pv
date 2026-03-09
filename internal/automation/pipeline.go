package automation

import (
	"os"

	"charm.land/huh/v2"
	"github.com/charmbracelet/x/term"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
)

// Context carries state through an automation pipeline.
type Context struct {
	ProjectPath string
	ProjectName string
	ProjectType string
	PHPVersion  string
	TLD         string
	Registry    *registry.Registry
	Settings    *config.Settings
	Env         map[string]string
	DBCreated   bool
}

// Step is a single automation action in a pipeline.
type Step interface {
	Label() string
	Gate() string
	ShouldRun(ctx *Context) bool
	Run(ctx *Context) (string, error)
}

// isInteractiveFunc detects whether stdin is a TTY. Swappable for tests.
var isInteractiveFunc = func() bool {
	return term.IsTerminal(os.Stdin.Fd())
}

// ConfirmFunc prompts the user for confirmation. Exported so service hooks
// in Tier 2 can reference it. Swappable for tests.
var ConfirmFunc = func(label string) bool {
	var confirmed bool
	err := huh.NewConfirm().
		Title(label + "?").
		Value(&confirmed).
		Run()
	if err != nil {
		return false
	}
	return confirmed
}

// RunPipeline executes a sequence of Steps, respecting each step's gate
// setting from the automation config.
func RunPipeline(steps []Step, ctx *Context) error {
	for _, step := range steps {
		if !step.ShouldRun(ctx) {
			continue
		}

		mode := LookupGate(&ctx.Settings.Automation, step.Gate())

		switch mode {
		case config.AutoOff:
			continue
		case config.AutoAsk:
			if !isInteractiveFunc() {
				continue
			}
			if !ConfirmFunc(step.Label()) {
				continue
			}
		}

		// AutoOn (or AutoAsk confirmed) — run the step.
		if err := ui.Step(step.Label(), func() (string, error) {
			return step.Run(ctx)
		}); err != nil {
			return err
		}
	}
	return nil
}

// LookupGate maps a gate string to its AutoMode value in the Automation config.
// Unknown gates default to AutoOn.
func LookupGate(a *config.Automation, gate string) config.AutoMode {
	switch gate {
	case "composer_install":
		return a.ComposerInstall
	case "copy_env":
		return a.CopyEnv
	case "generate_key":
		return a.GenerateKey
	case "set_app_url":
		return a.SetAppURL
	case "install_octane":
		return a.InstallOctane
	case "create_database":
		return a.CreateDatabase
	case "run_migrations":
		return a.RunMigrations
	case "update_env_on_service":
		return a.ServiceEnvUpdate
	case "service_fallback":
		return a.ServiceFallback
	default:
		return config.AutoOn
	}
}
