package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var colimaPathRemove bool

var colimaPathCmd = &cobra.Command{
	Use:   "colima:path",
	Short: "Expose or remove Colima from PATH",
	RunE: func(cmd *cobra.Command, args []string) error {
		t := tools.MustGet("colima")

		if colimaPathRemove {
			if err := tools.Unexpose(t); err != nil {
				return err
			}
			ui.Success("Colima removed from PATH")
			return nil
		}

		if err := tools.Expose(t); err != nil {
			return fmt.Errorf("cannot expose Colima: %w", err)
		}
		ui.Success("Colima added to PATH")
		return nil
	},
}

func init() {
	colimaPathCmd.Flags().BoolVar(&colimaPathRemove, "remove", false, "Remove from PATH instead of adding")
	rootCmd.AddCommand(colimaPathCmd)
}
