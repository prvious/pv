package mago

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var pathRemove bool

var pathCmd = &cobra.Command{
	Use:     "mago:path",
	GroupID: "mago",
	Short: "Expose or remove Mago from PATH",
	RunE: func(cmd *cobra.Command, args []string) error {
		t := tools.MustGet("mago")

		if pathRemove {
			if err := tools.Unexpose(t); err != nil {
				return err
			}
			ui.Success("Mago removed from PATH")
			return nil
		}

		if err := tools.Expose(t); err != nil {
			return fmt.Errorf("cannot expose Mago: %w", err)
		}
		ui.Success("Mago added to PATH")
		return nil
	},
}

func init() {
	pathCmd.Flags().BoolVar(&pathRemove, "remove", false, "Remove from PATH instead of adding")
}
