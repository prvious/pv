package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/setup"
	"github.com/spf13/cobra"
)

var (
	forceInstall bool
	installTLD   string
	installPHP   string
)

var installCmd = &cobra.Command{
	Use:   "install",
	Short: "Set up pv for the first time",
	RunE: func(cmd *cobra.Command, args []string) error {
		// 0. Validate TLD.
		if err := config.ValidateTLD(installTLD); err != nil {
			return err
		}

		// 1. Check prerequisites.
		fmt.Println("Checking prerequisites...")
		if err := setup.CheckOS(); err != nil {
			return err
		}
		fmt.Printf("  ✓ macOS detected (%s)\n", setup.PlatformLabel())

		if setup.IsAlreadyInstalled() && !forceInstall {
			return fmt.Errorf("pv is already installed at %s\n  Run with --force to reinstall", config.PvDir())
		}

		// 2. Create directory structure.
		fmt.Println("\nCreating directory structure...")
		if err := config.EnsureDirs(); err != nil {
			return fmt.Errorf("cannot create directories: %w", err)
		}
		fmt.Println("  ✓ ~/.pv directories created")

		// Save TLD setting.
		settings := &config.Settings{TLD: installTLD}
		if err := settings.Save(); err != nil {
			return fmt.Errorf("cannot save settings: %w", err)
		}
		fmt.Printf("  ✓ TLD set to .%s\n", installTLD)

		client := &http.Client{}

		// 3. Install PHP version via phpenv.
		phpVersion := installPHP
		if phpVersion == "" {
			fmt.Println("\nDetecting available PHP versions...")
			available, err := phpenv.AvailableVersions(client)
			if err != nil {
				return fmt.Errorf("cannot detect available PHP versions: %w", err)
			}
			if len(available) == 0 {
				return fmt.Errorf("no PHP versions found in releases")
			}
			// Pick the highest available version.
			phpVersion = available[len(available)-1]
			fmt.Printf("  Latest available: PHP %s\n", phpVersion)
		}

		fmt.Printf("\nInstalling PHP %s...\n", phpVersion)
		if err := phpenv.Install(client, phpVersion); err != nil {
			return fmt.Errorf("cannot install PHP %s: %w", phpVersion, err)
		}

		// Set as global default.
		if err := phpenv.SetGlobal(phpVersion); err != nil {
			return fmt.Errorf("cannot set global PHP: %w", err)
		}
		fmt.Printf("  ✓ PHP %s set as global default\n", phpVersion)

		// 4. Download other tools (Mago, Composer).
		fmt.Println("\nDownloading tools...")
		vs, err := binaries.LoadVersions()
		if err != nil {
			return fmt.Errorf("cannot load version state: %w", err)
		}

		for _, b := range binaries.Tools() {
			latest, err := binaries.FetchLatestVersion(client, b)
			if err != nil {
				return fmt.Errorf("cannot check %s version: %w", b.DisplayName, err)
			}

			if err := binaries.InstallBinary(client, b, latest); err != nil {
				return fmt.Errorf("cannot install %s: %w", b.DisplayName, err)
			}

			vs.Set(b.Name, latest)
		}

		// Write php shim.
		fmt.Println("\nWriting PHP shim...")
		if err := phpenv.WriteShims(); err != nil {
			return fmt.Errorf("cannot write shims: %w", err)
		}
		fmt.Println("  ✓ PHP shim created")

		// 5. Write version manifest.
		fmt.Println("\nWriting version manifest...")
		vs.Set("php", phpVersion)
		if err := vs.Save(); err != nil {
			return fmt.Errorf("cannot save versions: %w", err)
		}
		fmt.Println("  ✓ versions.json saved")

		// 6. Create main Caddyfile.
		fmt.Println("\nGenerating Caddyfile...")
		if err := caddy.GenerateCaddyfile(); err != nil {
			return fmt.Errorf("cannot generate Caddyfile: %w", err)
		}
		fmt.Println("  ✓ Caddyfile created")

		// 7. Create empty registry.
		fmt.Println("\nInitializing registry...")
		reg := &registry.Registry{}
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}
		fmt.Println("  ✓ registry.json created")

		// 8a. DNS resolver (sudo).
		fmt.Println("\nSetting up DNS resolver...")
		fmt.Println("  This requires administrator privileges.")
		if err := setup.RunSudoResolver(installTLD); err != nil {
			fmt.Printf("  x DNS resolver setup failed: %v\n", err)
			fmt.Println("  You can set this up manually later:")
			fmt.Println("    sudo mkdir -p /etc/resolver")
			fmt.Printf("    echo 'nameserver 127.0.0.1' | sudo tee /etc/resolver/%s\n", installTLD)
		} else {
			fmt.Println("  ✓ DNS resolver configured")
		}

		// 8b. Trust CA certificate (start server, trust, stop).
		fmt.Println("\nTrusting CA certificate...")
		if err := setup.RunSudoTrustWithServer(); err != nil {
			fmt.Printf("  x CA trust failed: %v\n", err)
			fmt.Println("  You can set this up manually later:")
			fmt.Printf("    %s/frankenphp run --config %s --adapter caddyfile &\n", config.BinDir(), config.CaddyfilePath())
			fmt.Printf("    sudo %s/frankenphp trust\n", config.BinDir())
			fmt.Println("    kill %%1")
		} else {
			fmt.Println("  ✓ Caddy CA certificate trusted")
		}

		// 9. Self-test.
		fmt.Println("\nRunning self-test...")
		results := setup.RunSelfTest(installTLD)
		setup.PrintResults(results)

		// 10. PATH instructions.
		fmt.Println()
		setup.PrintPathInstructions()

		// 11. Summary.
		fmt.Println()
		fmt.Println("pv installed!")
		fmt.Println()
		fmt.Printf("  PHP:        %s (global default)\n", phpVersion)
		for _, b := range binaries.Tools() {
			v := vs.Get(b.Name)
			if v == "" {
				v = "unknown"
			}
			fmt.Printf("  %-12s %s\n", b.DisplayName+":", v)
		}
		fmt.Println()
		fmt.Println("Install additional PHP versions with: pv php install <version>")
		fmt.Println("Run `pv link .` in a project to get started.")

		return nil
	},
}

func init() {
	installCmd.Flags().BoolVar(&forceInstall, "force", false, "Reinstall even if already installed")
	installCmd.Flags().StringVar(&installTLD, "tld", "test", "Top-level domain for local sites (e.g., test, pv-test)")
	installCmd.Flags().StringVar(&installPHP, "php", "", "PHP version to install (e.g., 8.4). Auto-detects latest if omitted.")
	rootCmd.AddCommand(installCmd)
}
