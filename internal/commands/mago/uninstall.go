package mago

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallCmd = &cobra.Command{
	Use:     "mago:uninstall",
	GroupID: "mago",
	Short: "Remove Mago binary and PATH entry",
	RunE: func(cmd *cobra.Command, args []string) error {
		return ui.Step("Removing Mago...", func() (string, error) {
			if err := tools.Unexpose(tools.MustGet("mago")); err != nil {
				return "", fmt.Errorf("cannot unexpose mago: %w", err)
			}

			if err := os.Remove(config.MagoPath()); err != nil && !os.IsNotExist(err) {
				return "", err
			}

			return "Mago removed", nil
		})
	},
}
