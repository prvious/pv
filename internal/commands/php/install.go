package php

import (
	"fmt"
	"net/http"
	"regexp"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var validPHPVersion = regexp.MustCompile(`^\d+\.\d+$`)

var installCmd = &cobra.Command{
	Use:     "php:install [version]",
	GroupID: "php",
	Short: "Install a PHP version (e.g., pv php:install 8.4). Installs latest if omitted.",
	Example: `# Install the latest PHP version
pv php:install

# Install a specific version
pv php:install 8.3`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := ""
		if len(args) > 0 {
			version = args[0]
		}

		// Auto-resolve latest if no version specified.
		if version == "" {
			client := &http.Client{}
			available, err := phpenv.AvailableVersions(client)
			if err != nil {
				return fmt.Errorf("cannot detect available PHP versions: %w", err)
			}
			if len(available) == 0 {
				return fmt.Errorf("no PHP versions found in releases")
			}
			version = available[len(available)-1]
		}

		if !validPHPVersion.MatchString(version) {
			return fmt.Errorf("invalid version format %q, use major.minor (e.g., 8.4)", version)
		}

		if phpenv.IsInstalled(version) {
			// Ensure global default is set even if already installed.
			if _, err := phpenv.GlobalVersion(); err != nil {
				if err := phpenv.SetGlobal(version); err != nil {
					return err
				}
			}
			ui.Success(fmt.Sprintf("PHP %s is already installed", version))
			return nil
		}

		// Download.
		if err := downloadCmd.RunE(downloadCmd, []string{version}); err != nil {
			return err
		}

		// If no global default, set this as the default.
		if _, err := phpenv.GlobalVersion(); err != nil {
			if err := phpenv.SetGlobal(version); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("PHP %s set as global default", version))
		}

		// Expose PHP and FrankenPHP to PATH.
		for _, name := range []string{"php", "frankenphp"} {
			t := tools.MustGet(name)
			if t.AutoExpose {
				if err := tools.Expose(t); err != nil {
					return fmt.Errorf("cannot expose %s: %w", name, err)
				}
			}
		}

		return nil
	},
}
