package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var phpUninstallCmd = &cobra.Command{
	Use:     "php:uninstall",
	GroupID: "php",
	Short: "Remove all PHP versions and PATH entries",
	RunE: func(cmd *cobra.Command, args []string) error {
		return ui.Step("Removing PHP...", func() (string, error) {
			for _, name := range []string{"php", "frankenphp"} {
				if err := tools.Unexpose(tools.MustGet(name)); err != nil {
					return "", fmt.Errorf("cannot unexpose %s: %w", name, err)
				}
			}

			if err := os.RemoveAll(config.PhpDir()); err != nil {
				return "", fmt.Errorf("cannot remove PHP directory: %w", err)
			}

			return "All PHP versions removed", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(phpUninstallCmd)
}
