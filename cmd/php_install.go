package cmd

import (
	"fmt"
	"net/http"
	"os"
	"regexp"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var validPHPVersion = regexp.MustCompile(`^\d+\.\d+$`)

var phpInstallCmd = &cobra.Command{
	Use:   "install <version>",
	Short: "Install a PHP version (e.g., pv php install 8.4)",
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
			ui.Success(fmt.Sprintf("PHP %s is already installed", ui.Green.Bold(true).Render(version)))
			fmt.Fprintln(os.Stderr)
			return nil
		}

		fmt.Fprintln(os.Stderr)

		client := &http.Client{}
		if err := ui.StepProgress("Installing PHP "+version+"...", func(progress func(written, total int64)) (string, error) {
			if err := phpenv.InstallProgress(client, version, progress); err != nil {
				return "", err
			}
			return fmt.Sprintf("PHP %s installed", version), nil
		}); err != nil {
			return err
		}

		// If no global default, set this as the default.
		if _, err := phpenv.GlobalVersion(); err != nil {
			if err := phpenv.SetGlobal(version); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("PHP %s set as global default", version))
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	phpCmd.AddCommand(phpInstallCmd)
}
