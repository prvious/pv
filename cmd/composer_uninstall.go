package cmd

import (
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var composerUninstallCmd = &cobra.Command{
	Use:   "composer:uninstall",
	Short: "Remove Composer PHAR, PATH entry, and global packages",
	RunE: func(cmd *cobra.Command, args []string) error {
		return ui.Step("Removing Composer...", func() (string, error) {
			t := tools.Get("composer")
			if t != nil {
				_ = tools.Unexpose(t)
			}

			if err := os.Remove(config.ComposerPharPath()); err != nil && !os.IsNotExist(err) {
				return "", err
			}

			if err := os.RemoveAll(config.ComposerDir()); err != nil {
				return "", err
			}

			return "Composer removed", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(composerUninstallCmd)
}
