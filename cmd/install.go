package cmd

import (
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var (
	forceInstall bool
	installTLD   string
	installWith  string
)

// withSpec holds parsed --with flag values.
type withSpec struct {
	phpVersion string // empty = latest
	mago       bool
	services   []serviceSpec
}

type serviceSpec struct {
	name    string
	version string
}

func parseWith(raw string) (withSpec, error) {
	var spec withSpec
	if raw == "" {
		return spec, nil
	}

	for _, item := range strings.Split(raw, ",") {
		item = strings.TrimSpace(item)
		if item == "" {
			continue
		}

		if strings.HasPrefix(item, "service[") && strings.HasSuffix(item, "]") {
			inner := item[8 : len(item)-1]
			parts := strings.SplitN(inner, ":", 2)
			s := serviceSpec{name: parts[0]}
			if len(parts) > 1 {
				s.version = parts[1]
			}
			if _, err := services.Lookup(s.name); err != nil {
				return spec, fmt.Errorf("unknown service %q in --with (available: %s)", s.name, strings.Join(services.Available(), ", "))
			}
			spec.services = append(spec.services, s)
		} else {
			parts := strings.SplitN(item, ":", 2)
			name := parts[0]
			version := ""
			if len(parts) > 1 {
				version = parts[1]
			}
			switch name {
			case "php":
				spec.phpVersion = version
			case "mago":
				spec.mago = true
			default:
				return spec, fmt.Errorf("unknown tool %q in --with (available: php, mago)", name)
			}
		}
	}
	return spec, nil
}

var installCmd = &cobra.Command{
	Use:   "install",
	Short: "Non-interactive setup — installs PHP, Composer, and configures the environment",
	Long: `Installs the core pv stack non-interactively. For an interactive setup wizard, use: pv setup

Non-negotiable tools (always installed): PHP, Composer
Optional tools: Mago (via --with)
Colima is installed automatically when you add your first service.

Examples:
  pv install
  pv install --tld=test
  pv install --with="php:8.2,mago"
  pv install --with="php:8.3,service[redis:7],service[mysql:8.0]"`,
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()

		spec, err := parseWith(installWith)
		if err != nil {
			return err
		}

		if err := config.ValidateTLD(installTLD); err != nil {
			return err
		}

		if setup.IsAlreadyInstalled() && !forceInstall {
			fmt.Fprintln(os.Stderr)
			ui.Fail("pv is already installed")
			ui.FailDetail("Run with --force to reinstall")
			fmt.Fprintln(os.Stderr)
			return ui.ErrAlreadyPrinted
		}

		ui.Header(version)

		if err := acquireSudo(); err != nil {
			return err
		}

		// Step 1: Check prerequisites.
		if err := ui.Step("Checking prerequisites...", func() (string, error) {
			if err := setup.CheckOS(); err != nil {
				return "", err
			}
			return fmt.Sprintf("macOS %s", setup.PlatformLabel()), nil
		}); err != nil {
			return ui.ErrAlreadyPrinted
		}

		// Step 2: Create directory structure and save settings.
		if err := ui.Step("Preparing environment...", func() (string, error) {
			if err := config.EnsureDirs(); err != nil {
				return "", fmt.Errorf("cannot create directories: %w", err)
			}
			settings, _ := config.LoadSettings()
			if settings == nil {
				settings = &config.Settings{}
			}
			settings.TLD = installTLD
			if err := settings.Save(); err != nil {
				return "", fmt.Errorf("cannot save settings: %w", err)
			}
			return "Directories created", nil
		}); err != nil {
			return ui.ErrAlreadyPrinted
		}

		// Step 3: Install PHP (non-negotiable).
		phpArgs := []string{}
		if spec.phpVersion != "" {
			phpArgs = []string{spec.phpVersion}
		}
		if err := phpInstallCmd.RunE(phpInstallCmd, phpArgs); err != nil {
			return ui.ErrAlreadyPrinted
		}

		// Step 4: Install Composer (non-negotiable).
		if err := composerInstallCmd.RunE(composerInstallCmd, nil); err != nil {
			return ui.ErrAlreadyPrinted
		}

		// Step 5: Install Mago (opt-in via --with).
		if spec.mago {
			if err := magoInstallCmd.RunE(magoInstallCmd, nil); err != nil {
				return ui.ErrAlreadyPrinted
			}
		}

		// Step 6: Finalize (Caddyfile, DNS, CA trust, shell PATH).
		if err := bootstrapFinalize(installTLD); err != nil {
			return ui.ErrAlreadyPrinted
		}

		// Step 7: Install services from --with.
		for _, svc := range spec.services {
			svcArgs := []string{svc.name}
			if svc.version != "" {
				svcArgs = append(svcArgs, svc.version)
			}
			if err := serviceAddCmd.RunE(serviceAddCmd, svcArgs); err != nil {
				fmt.Fprintf(os.Stderr, "  %s Service %s failed: %v\n", ui.Red.Render("!"), svc.name, err)
			}
		}

		ui.Footer(start, "https://pv.prvious.dev/docs")

		return nil
	},
}

// shortPath returns the path relative to HOME for display.
func shortPath(path string) string {
	home, _ := os.UserHomeDir()
	if strings.HasPrefix(path, home) {
		return path[len(home)+1:]
	}
	return path
}

func init() {
	installCmd.Flags().BoolVarP(&forceInstall, "force", "f", false, "Reinstall even if already installed")
	installCmd.SilenceUsage = true
	installCmd.Flags().StringVar(&installTLD, "tld", "test", "Top-level domain for local sites (e.g., test, pv-test)")
	installCmd.Flags().StringVar(&installWith, "with", "", `Optional tools and services (e.g., "php:8.2,mago,service[redis:7]")`)
	rootCmd.AddCommand(installCmd)
}
