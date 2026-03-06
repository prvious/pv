package cmd

import (
	"fmt"
	"net/http"
	"os"
	"strings"

	"github.com/charmbracelet/huh"
	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var setupCmd = &cobra.Command{
	Use:   "setup",
	Short: "Interactive setup wizard — choose PHP versions, tools, and services",
	RunE: func(cmd *cobra.Command, args []string) error {
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

		// Tool options.
		type toolChoice struct {
			Name    string
			Label   string
			Checked bool
		}
		toolDefs := []toolChoice{
			{"composer", "Composer", true},
			{"mago", "Mago", isExecutable(config.BinDir() + "/mago")},
			{"colima", "Colima (container runtime for services)", colima.IsInstalled()},
		}

		var toolOptions []huh.Option[string]
		var selectedTools []string
		for _, t := range toolDefs {
			label := t.Label
			toolOptions = append(toolOptions, huh.NewOption(label, t.Name))
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
					Title("Tools").
					Description("Select which tools to install").
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

		// Ensure directories exist.
		if err := config.EnsureDirs(); err != nil {
			return fmt.Errorf("cannot create directories: %w", err)
		}

		// Save TLD.
		if err := config.ValidateTLD(tld); err != nil {
			return err
		}
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
				fmt.Fprintf(os.Stderr, "  %s %s\n", ui.Red.Render("!"), fmt.Sprintf("PHP %s failed: %v", v, err))
			}
		}

		// Set global PHP if not set.
		if _, err := phpenv.GlobalVersion(); err != nil && len(selectedPHP) > 0 {
			latest := selectedPHP[len(selectedPHP)-1]
			if err := phpenv.SetGlobal(latest); err == nil {
				ui.Success(fmt.Sprintf("PHP %s set as global default", latest))
			}
		}

		// Install tools.
		toolSet := make(map[string]bool)
		for _, t := range selectedTools {
			toolSet[t] = true
		}

		if toolSet["composer"] {
			if err := ui.StepProgress("Installing Composer...", func(progress func(written, total int64)) (string, error) {
				vs, _ := binaries.LoadVersions()
				latest, err := binaries.FetchLatestVersion(client, binaries.Composer)
				if err != nil {
					return "", err
				}
				if err := binaries.InstallBinaryProgress(client, binaries.Composer, latest, progress); err != nil {
					return "", err
				}
				if vs != nil {
					vs.Set("composer", latest)
					_ = vs.Save()
				}
				return "Composer installed", nil
			}); err != nil {
				fmt.Fprintf(os.Stderr, "  %s Composer failed: %v\n", ui.Red.Render("!"), err)
			}
		}

		if toolSet["mago"] {
			if err := ui.StepProgress("Installing Mago...", func(progress func(written, total int64)) (string, error) {
				vs, _ := binaries.LoadVersions()
				latest, err := binaries.FetchLatestVersion(client, binaries.Mago)
				if err != nil {
					return "", err
				}
				if err := binaries.InstallBinaryProgress(client, binaries.Mago, latest, progress); err != nil {
					return "", err
				}
				if vs != nil {
					vs.Set("mago", latest)
					_ = vs.Save()
				}
				return fmt.Sprintf("Mago %s installed", latest), nil
			}); err != nil {
				fmt.Fprintf(os.Stderr, "  %s Mago failed: %v\n", ui.Red.Render("!"), err)
			}
		}

		if toolSet["colima"] {
			if err := ui.StepProgress("Installing Colima...", func(progress func(written, total int64)) (string, error) {
				if err := colima.Install(client, progress); err != nil {
					return "", err
				}
				return "Colima installed", nil
			}); err != nil {
				fmt.Fprintf(os.Stderr, "  %s Colima failed: %v\n", ui.Red.Render("!"), err)
			}
		}

		// Write shims.
		if err := phpenv.WriteShims(); err != nil {
			fmt.Fprintf(os.Stderr, "  %s Shims failed: %v\n", ui.Red.Render("!"), err)
		}

		// Print summary.
		fmt.Fprintln(os.Stderr)
		if len(selectedPHP) > 0 {
			ui.Success(fmt.Sprintf("PHP: %s", strings.Join(selectedPHP, ", ")))
		}
		if len(selectedTools) > 0 {
			ui.Success(fmt.Sprintf("Tools: %s", strings.Join(selectedTools, ", ")))
		}
		if len(selectedServices) > 0 {
			fmt.Fprintln(os.Stderr)
			ui.Subtle("To start your selected services, run:")
			for _, name := range selectedServices {
				fmt.Fprintf(os.Stderr, "  pv service:add %s\n", name)
			}
		}
		fmt.Fprintln(os.Stderr)

		return nil
	},
}

func init() {
	rootCmd.AddCommand(setupCmd)
}
