package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/setup"
	"github.com/spf13/cobra"
)

var (
	forceInstall bool
	installTLD   string
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

		// 3. Download binaries.
		fmt.Println("\nDownloading binaries...")
		client := &http.Client{}
		vs, err := binaries.LoadVersions()
		if err != nil {
			return fmt.Errorf("cannot load version state: %w", err)
		}

		for _, b := range binaries.All() {
			latest, err := binaries.FetchLatestVersion(client, b)
			if err != nil {
				return fmt.Errorf("cannot check %s version: %w", b.DisplayName, err)
			}

			version := latest
			if b.Name == "frankenphp" && len(version) > 0 && version[0] == 'v' {
				version = version[1:]
			}

			if err := binaries.InstallBinary(client, b, version); err != nil {
				return fmt.Errorf("cannot install %s: %w", b.DisplayName, err)
			}

			vs.Set(b.Name, latest)
		}

		// 3b. Install PHP CLI (version derived from FrankenPHP).
		fmt.Println("\nInstalling PHP CLI...")
		phpVersion, err := binaries.DetectPHPVersion(config.BinDir())
		if err != nil {
			return fmt.Errorf("cannot detect PHP version from FrankenPHP: %w", err)
		}
		if err := binaries.InstallBinary(client, binaries.PHP, phpVersion); err != nil {
			return fmt.Errorf("cannot install PHP CLI: %w", err)
		}
		vs.Set("php", phpVersion)
		fmt.Printf("  ✓ PHP CLI %s installed\n", phpVersion)

		// 4. Generate shims.
		fmt.Println("\nGenerating shims...")
		if err := binaries.WriteAllShims(); err != nil {
			return fmt.Errorf("cannot write shims: %w", err)
		}
		fmt.Println("  ✓ composer shim created")

		// 5. Write version manifest.
		fmt.Println("\nWriting version manifest...")
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
		printInstallSummary(vs)

		return nil
	},
}

func printInstallSummary(vs *binaries.VersionState) {
	fmt.Println("pv installed!")
	fmt.Println()
	// All() binaries plus PHP (which has a special install flow).
	summaryBinaries := append(binaries.All(), binaries.PHP)
	for _, b := range summaryBinaries {
		v := vs.Get(b.Name)
		if v == "" {
			v = "unknown"
		}
		fmt.Printf("  %-12s %s\n", b.DisplayName, v)
	}
	fmt.Println()
	fmt.Println("Run `pv link .` in a project to get started.")
}

func init() {
	installCmd.Flags().BoolVar(&forceInstall, "force", false, "Reinstall even if already installed")
	installCmd.Flags().StringVar(&installTLD, "tld", "test", "Top-level domain for local sites (e.g., test, pv-test)")
	rootCmd.AddCommand(installCmd)
}
