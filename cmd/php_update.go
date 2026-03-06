package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var phpUpdateCmd = &cobra.Command{
	Use:   "php:update",
	Short: "Re-download all installed PHP versions with the latest builds",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		versions, err := phpenv.InstalledVersions()
		if err != nil {
			return fmt.Errorf("cannot list installed versions: %w", err)
		}

		if len(versions) == 0 {
			ui.Success("No PHP versions installed")
			return nil
		}

		for _, v := range versions {
			if err := ui.StepProgress(fmt.Sprintf("Updating PHP %s...", v), func(progress func(written, total int64)) (string, error) {
				if err := phpenv.InstallProgress(client, v, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("PHP %s updated", v), nil
			}); err != nil {
				return err
			}
		}

		// Re-expose only if already on PATH.
		for _, name := range []string{"php", "frankenphp"} {
			t := tools.Get(name)
			if t != nil && tools.IsExposed(t) {
				_ = tools.Expose(t)
			}
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(phpUpdateCmd)
}
