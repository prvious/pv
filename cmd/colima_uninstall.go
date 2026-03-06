package cmd

import (
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var colimaUninstallCmd = &cobra.Command{
	Use:   "colima:uninstall",
	Short: "Stop Colima VM and remove the binary",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !colima.IsInstalled() {
			ui.Success("Colima not installed")
			return nil
		}

		return ui.Step("Removing Colima...", func() (string, error) {
			if colima.IsRunning() {
				_ = colima.Stop()
				_ = colima.Delete()
			}

			if err := os.Remove(config.ColimaPath()); err != nil && !os.IsNotExist(err) {
				return "", err
			}

			t := tools.Get("colima")
			if t != nil {
				_ = tools.Unexpose(t)
			}

			return "Colima removed", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(colimaUninstallCmd)
}
