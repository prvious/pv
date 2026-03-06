package cmd

import (
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/setup"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var (
	forceInstall   bool
	installTLD     string
	installPHP     string
	installVerbose bool
)

var installCmd = &cobra.Command{
	Use:   "install",
	Short: "Non-interactive setup — installs PHP, Composer, Mago, and Colima",
	Long:  "Installs the core pv stack non-interactively. For an interactive setup wizard, use: pv setup",
	RunE: func(cmd *cobra.Command, args []string) error {
		start := time.Now()

		// Propagate verbose flag.
		binaries.Verbose = installVerbose
		phpenv.Verbose = installVerbose
		setup.Verbose = installVerbose

		// Print header.
		ui.Header(version)

		// 0. Validate TLD.
		if err := config.ValidateTLD(installTLD); err != nil {
			return err
		}

		if setup.IsAlreadyInstalled() && !forceInstall {
			return fmt.Errorf("pv is already installed at %s\n  Run with --force to reinstall", config.PvDir())
		}

		// Acquire sudo credentials upfront so password prompt doesn't
		// interfere with spinner output during DNS/CA steps.
		ui.Subtle("pv needs sudo for DNS and certificate setup.")
		sudoCmd := exec.Command("sudo", "-v")
		sudoCmd.Stdin = os.Stdin
		sudoCmd.Stdout = os.Stderr
		sudoCmd.Stderr = os.Stderr
		if err := sudoCmd.Run(); err != nil {
			return fmt.Errorf("sudo authentication failed: %w", err)
		}
		fmt.Fprintln(os.Stderr)

		client := &http.Client{}

		// Step 1: Check prerequisites.
		if err := ui.Step("Checking prerequisites...", func() (string, error) {
			if err := setup.CheckOS(); err != nil {
				return "", err
			}
			return fmt.Sprintf("macOS %s", setup.PlatformLabel()), nil
		}); err != nil {
			return err
		}

		// Step 2: Install PHP (with progress bar for large downloads).
		phpVersion := installPHP
		var fullPHPResult string
		if err := ui.StepProgress("Installing PHP...", func(progress func(written, total int64)) (string, error) {
			if phpVersion == "" {
				available, err := phpenv.AvailableVersions(client)
				if err != nil {
					return "", fmt.Errorf("cannot detect available PHP versions: %w", err)
				}
				if len(available) == 0 {
					return "", fmt.Errorf("no PHP versions found in releases")
				}
				phpVersion = available[len(available)-1]
			}

			// Create directory structure first.
			if err := config.EnsureDirs(); err != nil {
				return "", fmt.Errorf("cannot create directories: %w", err)
			}

			// Save TLD setting.
			settings := &config.Settings{TLD: installTLD}
			if err := settings.Save(); err != nil {
				return "", fmt.Errorf("cannot save settings: %w", err)
			}

			// Install PHP version with progress tracking.
			if err := phpenv.InstallProgress(client, phpVersion, progress); err != nil {
				return "", fmt.Errorf("cannot install PHP %s: %w", phpVersion, err)
			}

			// Set as global default.
			if err := phpenv.SetGlobal(phpVersion); err != nil {
				return "", fmt.Errorf("cannot set global PHP: %w", err)
			}

			// Detect full version for display.
			fullVersion, err := binaries.DetectPHPVersion(config.PhpVersionDir(phpVersion))
			if err != nil {
				fullPHPResult = fmt.Sprintf("PHP %s (FrankenPHP + CLI)", phpVersion)
			} else {
				fullPHPResult = fmt.Sprintf("PHP %s (FrankenPHP + CLI)", fullVersion)
			}
			return fullPHPResult, nil
		}); err != nil {
			return err
		}

		// Step 3: Install tools (Mago, Composer).
		var toolVersions []string
		if err := ui.Step("Installing tools...", func() (string, error) {
			vs, err := binaries.LoadVersions()
			if err != nil {
				return "", fmt.Errorf("cannot load version state: %w", err)
			}

			for _, b := range binaries.Tools() {
				latest, err := binaries.FetchLatestVersion(client, b)
				if err != nil {
					return "", fmt.Errorf("cannot check %s version: %w", b.DisplayName, err)
				}

				if err := binaries.InstallBinary(client, b, latest); err != nil {
					return "", fmt.Errorf("cannot install %s: %w", b.DisplayName, err)
				}

				vs.Set(b.Name, latest)
				displayVersion := latest
				if displayVersion == "latest" {
					displayVersion = "installed"
				}
				toolVersions = append(toolVersions, fmt.Sprintf("%s %s", b.DisplayName, displayVersion))
			}

			// Migrate old composer.phar location.
			oldComposer := filepath.Join(config.DataDir(), "composer.phar")
			if _, err := os.Stat(oldComposer); err == nil {
				os.Remove(oldComposer)
			}

			// Expose tools (shims + symlinks).
			if err := tools.ExposeAll(); err != nil {
				return "", fmt.Errorf("cannot expose tools: %w", err)
			}

			// Migrate existing Composer config if present.
			setup.MigrateComposerConfig()

			// Save version manifest.
			vs.Set("php", phpVersion)
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			return strings.Join(toolVersions, ", "), nil
		}); err != nil {
			return err
		}

		// Step 4: Install Colima (container runtime for services).
		if err := ui.StepProgress("Installing Colima...", func(progress func(written, total int64)) (string, error) {
			if err := colima.Install(client, progress); err != nil {
				return "", fmt.Errorf("cannot install Colima: %w", err)
			}
			return "Colima installed", nil
		}); err != nil {
			// Colima install failure is non-fatal — services are optional.
			fmt.Fprintf(os.Stderr, "  %s %s\n", ui.Muted.Render("!"), ui.Muted.Render(fmt.Sprintf("Colima install skipped: %v", err)))
		}

		// Step 5: Configure environment.
		if err := ui.Step("Configuring environment...", func() (string, error) {
			// Generate Caddyfile.
			if err := caddy.GenerateCaddyfile(); err != nil {
				return "", fmt.Errorf("cannot generate Caddyfile: %w", err)
			}

			// Create empty registry.
			reg := &registry.Registry{}
			if err := reg.Save(); err != nil {
				return "", fmt.Errorf("cannot save registry: %w", err)
			}

			return "Environment configured", nil
		}); err != nil {
			return err
		}

		// Step 6: DNS resolver (sudo).
		if err := ui.Step("Setting up DNS resolver...", func() (string, error) {
			if err := setup.RunSudoResolver(installTLD); err != nil {
				return "", fmt.Errorf("DNS resolver setup failed: %w", err)
			}
			return "DNS resolver configured", nil
		}); err != nil {
			return err
		}

		// Step 7: Trust CA certificate (sudo).
		if err := ui.Step("Trusting HTTPS certificate...", func() (string, error) {
			if err := setup.RunSudoTrustWithServer(); err != nil {
				return "", fmt.Errorf("CA trust failed: %w", err)
			}
			return "HTTPS certificate trusted", nil
		}); err != nil {
			return err
		}

		// Step 8: Self-test.
		if err := ui.Step("Running self-test...", func() (string, error) {
			results := setup.RunSelfTest(installTLD)
			var failures []string
			for _, r := range results {
				if r.Err != nil {
					failures = append(failures, fmt.Sprintf("%s: %v", r.Name, r.Err))
				}
			}
			if len(failures) > 0 {
				return "", fmt.Errorf("self-test failures:\n    %s", strings.Join(failures, "\n    "))
			}
			return "All checks passed", nil
		}); err != nil {
			return err
		}

		// Step 9: Shell PATH.
		if err := ui.Step("Configuring shell...", func() (string, error) {
			shell := setup.DetectShell()
			configFile := setup.ShellConfigFile(shell)
			line := setup.PathExportLine(shell)

			// Check if already in PATH.
			data, err := os.ReadFile(configFile)
			if err == nil && strings.Contains(string(data), line) {
				return "PATH already configured", nil
			}

			// Add to config file.
			f, err := os.OpenFile(configFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
			if err != nil {
				return "", fmt.Errorf("cannot open %s: %w", configFile, err)
			}
			defer f.Close()
			if _, err := fmt.Fprintf(f, "\n# pv\n%s\n", line); err != nil {
				return "", fmt.Errorf("cannot write to %s: %w", configFile, err)
			}

			return fmt.Sprintf("Added to ~/%s", shortPath(configFile)), nil
		}); err != nil {
			return err
		}

		// Footer.
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
	installCmd.Flags().BoolVar(&forceInstall, "force", false, "Reinstall even if already installed")
	installCmd.Flags().StringVar(&installTLD, "tld", "test", "Top-level domain for local sites (e.g., test, pv-test)")
	installCmd.Flags().StringVar(&installPHP, "php", "", "PHP version to install (e.g., 8.4). Auto-detects latest if omitted.")
	installCmd.Flags().BoolVarP(&installVerbose, "verbose", "v", false, "Show detailed output")
	rootCmd.AddCommand(installCmd)
}
