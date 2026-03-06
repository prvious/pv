package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var colimaUpdateCmd = &cobra.Command{
	Use:   "colima:update",
	Short: "Update Colima to the latest version",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !colima.IsInstalled() {
			ui.Success("Colima not installed (run: pv colima:install)")
			return nil
		}

		client := &http.Client{}

		return ui.StepProgress("Updating Colima...", func(progress func(written, total int64)) (string, error) {
			if err := colima.Install(client, progress); err != nil {
				return "", fmt.Errorf("cannot download Colima: %w", err)
			}

			// Re-expose if already on PATH.
			t := tools.Get("colima")
			if t != nil && tools.IsExposed(t) {
				if err := tools.Expose(t); err != nil {
					return "", fmt.Errorf("cannot expose Colima: %w", err)
				}
			}

			return "Colima updated", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(colimaUpdateCmd)
}
