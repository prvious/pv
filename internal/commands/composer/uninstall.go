package composer

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallCmd = &cobra.Command{
	Use:     "composer:uninstall",
	GroupID: "composer",
	Short: "Remove Composer PHAR, PATH entry, and global packages",
	RunE: func(cmd *cobra.Command, args []string) error {
		return ui.Step("Removing Composer...", func() (string, error) {
			if err := tools.Unexpose(tools.MustGet("composer")); err != nil {
				return "", fmt.Errorf("cannot unexpose composer: %w", err)
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
