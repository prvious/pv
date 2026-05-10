package cmd

import (
	"errors"
	"fmt"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/prvious/pv/internal/commands/composer"
	daemoncmds "github.com/prvious/pv/internal/commands/daemon"
	"github.com/prvious/pv/internal/commands/mago"
	mailpitcmd "github.com/prvious/pv/internal/commands/mailpit"
	"github.com/prvious/pv/internal/commands/php"
	rustfscmd "github.com/prvious/pv/internal/commands/rustfs"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/packages"
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
	name string
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
			name := item[8 : len(item)-1]
			if !isKnownBinaryAddon(name) {
				return spec, fmt.Errorf("unknown service %q in --with (available: %s)", name, strings.Join(binaryAddons, ", "))
			}
			spec.services = append(spec.services, serviceSpec{name: name})
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
	Use:     "install",
	GroupID: "core",
	Short:   "Non-interactive setup — installs PHP, Composer, and configures the environment",
	Long: `Installs the core pv stack non-interactively. For an interactive setup wizard, use: pv setup

Non-negotiable tools (always installed): PHP, Composer
Optional tools: Mago (via --with)`,
	Example: `# Install with defaults
pv install

# Specify a custom TLD
pv install --tld=test

# Choose a specific PHP version and optional tools
pv install --with="php:8.2,mago"

# Include backing services (s3, mail)
pv install --with="php:8.3,service[s3],service[mail]"`,
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
			return fmt.Errorf("pv is already installed, run with --force to reinstall")
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
			return err
		}

		// Step 2: Create directory structure and save settings.
		if err := ui.Step("Preparing environment...", func() (string, error) {
			if err := config.EnsureDirs(); err != nil {
				return "", fmt.Errorf("cannot create directories: %w", err)
			}
			settings, err := config.LoadSettings()
			if err != nil {
				return "", fmt.Errorf("cannot load settings: %w", err)
			}
			settings.Defaults.TLD = installTLD
			if err := settings.Save(); err != nil {
				return "", fmt.Errorf("cannot save settings: %w", err)
			}
			return "Directories created", nil
		}); err != nil {
			return err
		}

		// Step 3: Install PHP (non-negotiable).
		phpArgs := []string{}
		if spec.phpVersion != "" {
			phpArgs = []string{spec.phpVersion}
		}
		if err := php.RunInstall(phpArgs); err != nil {
			return err
		}

		// Step 4: Install Composer (non-negotiable).
		if err := composer.RunInstall(); err != nil {
			return err
		}

		// Migrate existing Composer credentials (auth.json, config.json) into
		// pv's isolated COMPOSER_HOME so private packages keep working.
		setup.MigrateComposerConfig()

		// Step 5: Install managed packages.
		pkgClient := &http.Client{}
		var pkgFailures []string
		for _, pkg := range packages.Managed {
			if pkg.Method == packages.MethodPHAR {
				if err := ui.StepProgress(fmt.Sprintf("Installing %s...", pkg.Name), func(progress func(written, total int64)) (string, error) {
					version, err := packages.Install(cmd.Context(), pkgClient, pkg, progress)
					if err != nil {
						return "", err
					}
					return fmt.Sprintf("%s %s", pkg.Name, version), nil
				}); err != nil {
					if !errors.Is(err, ui.ErrAlreadyPrinted) {
						ui.Fail(fmt.Sprintf("%s install failed: %v", pkg.Name, err))
					}
					pkgFailures = append(pkgFailures, pkg.Name)
				}
			} else {
				if err := ui.Step(fmt.Sprintf("Installing %s...", pkg.Name), func() (string, error) {
					version, err := packages.Install(cmd.Context(), pkgClient, pkg, nil)
					if err != nil {
						return "", err
					}
					return fmt.Sprintf("%s %s", pkg.Name, version), nil
				}); err != nil {
					if !errors.Is(err, ui.ErrAlreadyPrinted) {
						ui.Fail(fmt.Sprintf("%s install failed: %v", pkg.Name, err))
					}
					pkgFailures = append(pkgFailures, pkg.Name)
				}
			}
		}
		if len(pkgFailures) > 0 {
			ui.Subtle(fmt.Sprintf("Warning: some packages failed to install: %s", strings.Join(pkgFailures, ", ")))
		}

		// Step 6: Install Mago (opt-in via --with).
		if spec.mago {
			if err := mago.RunInstall(); err != nil {
				return err
			}
		}

		// Step 7: Finalize (Caddyfile, DNS, CA trust, shell PATH).
		if err := bootstrapFinalize(installTLD); err != nil {
			return err
		}

		// Step 8: Enable daemon unless explicitly disabled in ~/.pv/pv.yml.
		settings, loadErr := config.LoadSettings()
		if loadErr != nil {
			ui.Subtle(fmt.Sprintf("Warning: could not load settings for daemon setup: %v", loadErr))
			settings = config.DefaultSettings()
		}
		if settings.Defaults.DaemonEnabled() {
			if err := daemoncmds.RunEnable(); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("Daemon setup failed: %v", err))
				}
				ui.Subtle("Run 'pv daemon:enable' to retry.")
			}
		}

		// Step 9: Install binary services from --with.
		for _, svc := range spec.services {
			if err := installBinaryService(svc.name); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("Service %s failed: %v", svc.name, err))
				}
			}
		}

		ui.Footer(start, "https://pv.prvious.dev/docs")

		return nil
	},
}

// binaryAddons is the canonical list of binary addon names accepted by --with.
var binaryAddons = []string{"mail", "s3"}

// isKnownBinaryAddon reports whether name is a recognised binary addon.
func isKnownBinaryAddon(name string) bool {
	for _, a := range binaryAddons {
		if a == name {
			return true
		}
	}
	return false
}

// installBinaryService installs a binary backing service by name.
// Only s3 (rustfs) and mail (mailpit) are supported. Database services
// (postgres, mysql, redis) have their own first-class commands.
func installBinaryService(name string) error {
	switch name {
	case "s3":
		return rustfscmd.RunInstall()
	case "mail":
		return mailpitcmd.RunInstall()
	default:
		return fmt.Errorf("unknown binary service %q", name)
	}
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
	installCmd.Flags().StringVar(&installTLD, "tld", "test", "Top-level domain for local sites (e.g., test, pv-test)")
	installCmd.Flags().StringVar(&installWith, "with", "", `Optional tools and services (e.g., "php:8.2,mago,service[redis:7]")`)
	rootCmd.AddCommand(installCmd)
}
