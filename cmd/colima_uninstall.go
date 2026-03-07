package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var colimaUninstallCmd = &cobra.Command{
	Use:     "colima:uninstall",
	GroupID: "colima",
	Short: "Stop Colima VM and remove the binary",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !colima.IsInstalled() {
			ui.Success("Colima not installed")
			return nil
		}

		return ui.Step("Removing Colima...", func() (string, error) {
			if colima.IsRunning() {
				if err := colima.Stop(); err != nil {
					return "", fmt.Errorf("cannot stop Colima VM (stop it manually before uninstalling): %w", err)
				}
				if err := colima.Delete(); err != nil {
					return "", fmt.Errorf("cannot delete Colima VM: %w", err)
				}
			}

			if err := os.Remove(config.ColimaPath()); err != nil && !os.IsNotExist(err) {
				return "", err
			}

			if err := tools.Unexpose(tools.MustGet("colima")); err != nil {
				return "", fmt.Errorf("cannot unexpose colima: %w", err)
			}

			return "Colima removed", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(colimaUninstallCmd)
}
