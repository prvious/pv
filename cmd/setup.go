package cmd

import (
	"errors"
	"fmt"
	"net/http"
	"os"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/commands/composer"
	daemoncmds "github.com/prvious/pv/internal/commands/daemon"
	"github.com/prvious/pv/internal/commands/mago"
	"github.com/prvious/pv/internal/commands/service"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/services"
	setupinternal "github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var setupCmd = &cobra.Command{
	Use:     "setup",
	GroupID: "core",
	Short:   "Interactive setup wizard — choose PHP versions, tools, and services",
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()
		client := &http.Client{}

		// Load current state to pre-select installed items.
		installedVersions, err := phpenv.InstalledVersions()
		if err != nil {
			ui.Subtle(fmt.Sprintf("Warning: could not detect installed PHP versions: %v", err))
		}
		installedSet := make(map[string]bool)
		for _, v := range installedVersions {
			installedSet[v] = true
		}

		// Fetch available PHP versions.
		available, err := phpenv.AvailableVersions(client)
		if err != nil {
			return fmt.Errorf("cannot fetch available PHP versions: %w", err)
		}

		// Build PHP version options.
		var phpOpts []selectOption
		for _, v := range available {
			label := "PHP " + v
			sel := installedSet[v]
			if installedSet[v] {
				label += " (installed)"
			}
			if len(installedVersions) == 0 && v == available[len(available)-1] {
				sel = true
			}
			phpOpts = append(phpOpts, selectOption{label: label, value: v, selected: sel})
		}

		// Tool options.
		toolOpts := []selectOption{
			{label: "Mago (PHP linter & formatter)", value: "mago", selected: isExecutable(config.BinDir() + "/mago")},
		}

		// Service options.
		var svcOpts []selectOption
		for _, name := range services.Available() {
			svc, _ := services.Lookup(name)
			if svc != nil {
				svcOpts = append(svcOpts, selectOption{label: svc.DisplayName(), value: name})
			}
		}

		// Load settings, falling back to defaults on error.
		settings, err := config.LoadSettings()
		if err != nil {
			ui.Subtle(fmt.Sprintf("Warning: could not load settings: %v", err))
			settings = config.DefaultSettings()
		}
		tld := settings.Defaults.TLD
		daemon := settings.Defaults.DaemonEnabled()
		automation := settings.Automation

		// Run the tabbed setup wizard.
		result, err := tea.NewProgram(
			newSetupModel(phpOpts, toolOpts, svcOpts, tld, daemon, automation),
			tea.WithOutput(os.Stderr),
		).Run()
		if err != nil {
			return fmt.Errorf("setup wizard failed: %w", err)
		}

		final, ok := result.(setupModel)
		if !ok {
			return fmt.Errorf("setup wizard returned unexpected state")
		}
		if !final.confirmed {
			return nil
		}

		selectedPHP := selectedValues(final.phpOptions)
		selectedTools := selectedValues(final.toolOptions)
		selectedServices := selectedValues(final.svcOptions)
		tld = final.tld
		daemon = final.daemon
		automation = final.automation

		fmt.Fprintln(os.Stderr)

		ui.Header(version)

		// Validate TLD.
		if err := config.ValidateTLD(tld); err != nil {
			return err
		}

		if err := ui.Step("Checking prerequisites...", func() (string, error) {
			if err := setupinternal.CheckOS(); err != nil {
				return "", err
			}
			return fmt.Sprintf("macOS %s", setupinternal.PlatformLabel()), nil
		}); err != nil {
			return err
		}

		// Acquire sudo upfront.
		if err := acquireSudo(); err != nil {
			return err
		}

		if err := ui.Step("Preparing environment...", func() (string, error) {
			if err := config.EnsureDirs(); err != nil {
				return "", fmt.Errorf("cannot create directories: %w", err)
			}

			// Build settings from wizard output, preserving existing PHP default.
			s := &config.Settings{
				Defaults:   config.Defaults{TLD: tld, PHP: settings.Defaults.PHP, Daemon: config.BoolPtr(daemon)},
				Automation: automation,
			}
			if err := s.Save(); err != nil {
				return "", fmt.Errorf("cannot save settings: %w", err)
			}

			// Write Valet-compatible config for Vite TLS auto-detection.
			if err := certs.EnsureValetConfig(tld); err != nil {
				ui.Subtle(fmt.Sprintf("Vite TLS config: %v", err))
			}

			return "Settings saved", nil
		}); err != nil {
			return err
		}

		// Install PHP versions.
		for _, v := range selectedPHP {
			if phpenv.IsInstalled(v) {
				ui.Success(fmt.Sprintf("PHP %s already installed", v))
				continue
			}
			if err := ui.StepProgress(fmt.Sprintf("Installing PHP %s...", v), func(progress func(written, total int64)) (string, error) {
				if err := phpenv.InstallProgress(client, v, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("PHP %s installed", v), nil
			}); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("PHP %s failed: %v", v, err))
				}
			}
		}

		if err := ui.Step("Configuring global PHP...", func() (string, error) {
			if len(selectedPHP) == 0 {
				return "No PHP versions selected", nil
			}

			if _, err := phpenv.GlobalVersion(); err == nil {
				return "Global PHP already configured", nil
			}

			latest := selectedPHP[len(selectedPHP)-1]
			if err := phpenv.SetGlobal(latest); err != nil {
				return "", err
			}

			return fmt.Sprintf("PHP %s set as global default", latest), nil
		}); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Global PHP setup failed: %v", err))
			}
		}

		// Install Composer (non-negotiable).
		if err := composer.RunInstall(); err != nil {
			if !errors.Is(err, ui.ErrAlreadyPrinted) {
				ui.Fail(fmt.Sprintf("Composer failed: %v", err))
			}
		}

		// Install optional tools (Colima is lazy-installed via service:add).
		toolSet := make(map[string]bool)
		for _, t := range selectedTools {
			toolSet[t] = true
		}

		if toolSet["mago"] {
			if err := mago.RunDownload(); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("Mago failed: %v", err))
				}
			}
		}

		if err := ui.Step("Updating tool shims...", func() (string, error) {
			if err := tools.ExposeAll(); err != nil {
				return "", err
			}

			vs, err := binaries.LoadVersions()
			if err == nil {
				if len(selectedPHP) > 0 {
					vs.Set("php", selectedPHP[len(selectedPHP)-1])
				}
				if saveErr := vs.Save(); saveErr != nil {
					ui.Fail(fmt.Sprintf("Cannot save version manifest: %v", saveErr))
				}
			}

			return "Tool shims updated", nil
		}); err != nil {
			ui.Fail(fmt.Sprintf("Tool exposure failed: %v", err))
		}

		// Finalize: Caddyfile, DNS, CA trust, shell PATH.
		if err := bootstrapFinalize(tld); err != nil {
			return err
		}

		// Enable daemon if selected.
		if daemon {
			if err := daemoncmds.RunEnable(); err != nil {
				if !errors.Is(err, ui.ErrAlreadyPrinted) {
					ui.Fail(fmt.Sprintf("Daemon setup failed: %v", err))
				}
				ui.Subtle("Run 'pv daemon:enable' to retry.")
			}
		}

		// Spin up selected services.
		if len(selectedServices) > 0 {
			fmt.Fprintln(os.Stderr)
			for _, name := range selectedServices {
				svc, _ := services.Lookup(name)
				if svc == nil {
					continue
				}
				svcArgs := []string{name, svc.DefaultVersion()}
				if err := service.RunAdd(svcArgs); err != nil {
					if !errors.Is(err, ui.ErrAlreadyPrinted) {
						ui.Fail(fmt.Sprintf("Service %s failed: %v", name, err))
					}
				}
			}
		}

		ui.Footer(start, "https://pv.prvious.dev/docs")

		return nil
	},
}

func init() {
	rootCmd.AddCommand(setupCmd)
}
