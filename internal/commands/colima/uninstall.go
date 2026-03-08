package colima

import (
	"fmt"
	"os"

	internalcolima "github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var uninstallCmd = &cobra.Command{
	Use:     "colima:uninstall",
	GroupID: "colima",
	Short:   "Stop Colima VM and remove the binary",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !internalcolima.IsInstalled() {
			ui.Success("Colima not installed")
			return nil
		}

		return ui.Step("Removing Colima...", func() (string, error) {
			if internalcolima.IsRunning() {
				if err := internalcolima.Stop(); err != nil {
					return "", fmt.Errorf("cannot stop Colima VM (stop it manually before uninstalling): %w", err)
				}
				if err := internalcolima.Delete(); err != nil {
					return "", fmt.Errorf("cannot delete Colima VM: %w", err)
				}
			}

			if err := os.Remove(config.ColimaPath()); err != nil && !os.IsNotExist(err) {
				return "", err
			}

			if err := internalcolima.RemoveLima(); err != nil && !os.IsNotExist(err) {
				return "", fmt.Errorf("cannot remove Lima: %w", err)
			}

			if err := tools.Unexpose(tools.MustGet("colima")); err != nil {
				return "", fmt.Errorf("cannot unexpose colima: %w", err)
			}

			return "Colima removed", nil
		})
	},
}
