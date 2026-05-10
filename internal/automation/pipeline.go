package automation

import (
	"fmt"
	"os"

	"charm.land/huh/v2"
	"github.com/charmbracelet/x/term"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
)

// Context carries state through an automation pipeline.
type Context struct {
	ProjectPath   string
	ProjectName   string
	ProjectType   string
	PHPVersion    string
	GlobalPHP     string
	TLD           string
	Registry      *registry.Registry
	Settings      *config.Settings
	Env           map[string]string
	DBCreated     bool
	ProjectConfig *config.ProjectConfig
}

// Step is a single automation action in a pipeline.
type Step interface {
	Label() string
	Gate() string
	Critical() bool
	ShouldRun(ctx *Context) bool
	Run(ctx *Context) (string, error)
}

// isInteractiveFunc detects whether stdin is a TTY. Swappable for tests.
var isInteractiveFunc = func() bool {
	return term.IsTerminal(os.Stdin.Fd())
}

// IsInteractive returns whether stdin is a TTY.
// Exported so service hooks can check before prompting.
func IsInteractive() bool {
	return isInteractiveFunc()
}

// ConfirmFunc prompts the user for confirmation. Exported so service hooks
// (internal/commands/service/hooks.go) can reuse the same confirmation flow.
// Swappable for tests.
var ConfirmFunc = func(label string) (bool, error) {
	var confirmed bool
	err := huh.NewConfirm().
		Title(label + "?").
		Value(&confirmed).
		Run()
	if err != nil {
		return false, err
	}
	return confirmed, nil
}

// RunPipeline executes a sequence of Steps, respecting each step's gate
// setting from the automation config. Non-critical step failures are displayed
// by ui.Step (✗) but do not abort the pipeline. Critical step failures abort
// immediately. The registry is saved at the end of a successful run.
func RunPipeline(steps []Step, ctx *Context) error {
	for _, step := range steps {
		if !step.ShouldRun(ctx) {
			continue
		}

		mode := LookupGate(&ctx.Settings.Automation, step.Gate())

		switch mode {
		case config.AutoOff:
			if step.Critical() {
				ui.Subtle(fmt.Sprintf("Skipped: %s (disabled in automation config)", step.Label()))
			}
			continue
		case config.AutoAsk:
			if !isInteractiveFunc() {
				if step.Critical() {
					ui.Subtle(fmt.Sprintf("Skipped: %s (non-interactive)", step.Label()))
				}
				continue
			}
			confirmed, err := ConfirmFunc(step.Label())
			if err != nil {
				return fmt.Errorf("automation prompt failed: %w", err)
			}
			if !confirmed {
				continue
			}
		}

		err := ui.Step(step.Label(), func() (string, error) {
			return step.Run(ctx)
		})
		if err != nil && step.Critical() {
			return err
		}
	}
	if ctx.Registry != nil {
		return ctx.Registry.Save()
	}
	return nil
}

// LookupGate maps a gate string to its AutoMode value in the Automation config.
// Unknown gates default to AutoAsk to avoid accidentally running unconfigured steps.
func LookupGate(a *config.Automation, gate string) config.AutoMode {
	switch gate {
	case "install_php_version":
		return a.InstallPHPVersion
	case "composer_install":
		return a.ComposerInstall
	case "copy_env":
		return a.CopyEnv
	case "generate_key":
		return a.GenerateKey
	case "set_app_url":
		return a.SetAppURL
	case "set_vite_tls":
		return a.SetViteTLS
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
	case "generate_site_config":
		return a.GenerateSiteConfig
	case "generate_caddyfile":
		return a.GenerateCaddyfile
	case "generate_tls_cert":
		return a.GenerateTLSCert
	case "detect_services":
		return a.DetectServices
	default:
		fmt.Fprintf(os.Stderr, "Warning: unknown automation gate %q, defaulting to ask\n", gate)
		return config.AutoAsk
	}
}
