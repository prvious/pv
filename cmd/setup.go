package cmd

import (
	"errors"
	"fmt"
	"net/http"
	"os"
	"time"

	"charm.land/huh/v2"
	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/commands/composer"
	"github.com/prvious/pv/internal/commands/mago"
	"github.com/prvious/pv/internal/commands/service"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/services"
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
		installedVersions, _ := phpenv.InstalledVersions()
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
		var phpOptions []huh.Option[string]
		for _, v := range available {
			label := "PHP " + v
			if installedSet[v] {
				label += " (installed)"
			}
			phpOptions = append(phpOptions, huh.NewOption(label, v))
		}

		// Pre-select: installed versions, or latest if none installed.
		var selectedPHP []string
		if len(installedVersions) > 0 {
			selectedPHP = append(selectedPHP, installedVersions...)
		} else if len(available) > 0 {
			selectedPHP = []string{available[len(available)-1]}
		}

		// Tool options (mago is opt-in, composer is non-negotiable but shown).
		type toolChoice struct {
			Name    string
			Label   string
			Checked bool
		}
		toolDefs := []toolChoice{
			{"mago", "Mago (PHP linter & formatter)", isExecutable(config.BinDir() + "/mago")},
		}

		var toolOptions []huh.Option[string]
		var selectedTools []string
		for _, t := range toolDefs {
			toolOptions = append(toolOptions, huh.NewOption(t.Label, t.Name))
			if t.Checked {
				selectedTools = append(selectedTools, t.Name)
			}
		}

		// Service options.
		var svcOptions []huh.Option[string]
		var selectedServices []string
		for _, name := range services.Available() {
			svc, _ := services.Lookup(name)
			if svc != nil {
				svcOptions = append(svcOptions, huh.NewOption(svc.DisplayName(), name))
			}
		}

		// TLD.
		settings, _ := config.LoadSettings()
		tld := "test"
		if settings != nil && settings.TLD != "" {
			tld = settings.TLD
		}

		// Run the form.
		form := huh.NewForm(
			huh.NewGroup(
				huh.NewMultiSelect[string]().
					Title("PHP Versions").
					Description("Select which PHP versions to install").
					Options(phpOptions...).
					Value(&selectedPHP),

				huh.NewMultiSelect[string]().
					Title("Optional Tools").
					Description("Composer is always installed. Select additional tools:").
					Options(toolOptions...).
					Value(&selectedTools),

				huh.NewMultiSelect[string]().
					Title("Services").
					Description("Select backing services to set up").
					Options(svcOptions...).
					Value(&selectedServices),

				huh.NewInput().
					Title("TLD").
					Description("Top-level domain for local sites").
					Value(&tld),
			),
		)

		if err := form.Run(); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)

		ui.Header(version)

		// Validate TLD.
		if err := config.ValidateTLD(tld); err != nil {
			return err
		}

		// Acquire sudo upfront.
		if err := acquireSudo(); err != nil {
			return err
		}

		// Ensure directories exist.
		if err := config.EnsureDirs(); err != nil {
			return fmt.Errorf("cannot create directories: %w", err)
		}

		// Save TLD.
		s := &config.Settings{TLD: tld}
		if settings != nil {
			s.GlobalPHP = settings.GlobalPHP
		}
		if err := s.Save(); err != nil {
			return fmt.Errorf("cannot save settings: %w", err)
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

		// Set global PHP if not set.
		if _, err := phpenv.GlobalVersion(); err != nil && len(selectedPHP) > 0 {
			latest := selectedPHP[len(selectedPHP)-1]
			if err := phpenv.SetGlobal(latest); err == nil {
				ui.Success(fmt.Sprintf("PHP %s set as global default", latest))
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

		// Expose all installed tools (shims + symlinks).
		if err := tools.ExposeAll(); err != nil {
			ui.Fail(fmt.Sprintf("Tool exposure failed: %v", err))
		}

		// Save version manifest.
		vs, err := binaries.LoadVersions()
		if err == nil {
			if len(selectedPHP) > 0 {
				vs.Set("php", selectedPHP[len(selectedPHP)-1])
			}
			if saveErr := vs.Save(); saveErr != nil {
				ui.Fail(fmt.Sprintf("Cannot save version manifest: %v", saveErr))
			}
		}

		// Finalize: Caddyfile, DNS, CA trust, shell PATH.
		if err := bootstrapFinalize(tld); err != nil {
			return err
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
