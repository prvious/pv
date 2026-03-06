package cmd

import (
	"fmt"
	"os"
	"regexp"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var validPHPVersion = regexp.MustCompile(`^\d+\.\d+$`)

var phpInstallCmd = &cobra.Command{
	Use:   "php:install <version>",
	Short: "Install a PHP version (e.g., pv php:install 8.4)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !validPHPVersion.MatchString(version) {
			fmt.Fprintln(os.Stderr)
			ui.Fail(fmt.Sprintf("Invalid version format %s", ui.Bold.Render(version)))
			ui.FailDetail("Use major.minor (e.g., 8.4)")
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
		}

		if phpenv.IsInstalled(version) {
			fmt.Fprintln(os.Stderr)
			ui.Success(fmt.Sprintf("PHP %s is already installed", version))
			fmt.Fprintln(os.Stderr)
			return nil
		}

		fmt.Fprintln(os.Stderr)

		// Download.
		if err := phpDownloadCmd.RunE(phpDownloadCmd, []string{version}); err != nil {
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
			t := tools.Get(name)
			if t != nil && t.AutoExpose {
				if err := tools.Expose(t); err != nil {
					return fmt.Errorf("cannot expose %s: %w", name, err)
				}
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(phpInstallCmd)
}
