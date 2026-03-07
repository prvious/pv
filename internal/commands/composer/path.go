package composer

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var pathRemove bool

var pathCmd = &cobra.Command{
	Use:     "composer:path",
	GroupID: "composer",
	Short:   "Expose or remove Composer from PATH",
	RunE: func(cmd *cobra.Command, args []string) error {
		t := tools.MustGet("composer")

		if pathRemove {
			if err := tools.Unexpose(t); err != nil {
				return err
			}
			ui.Success("Composer removed from PATH")
			return nil
		}

		if err := tools.Expose(t); err != nil {
			return fmt.Errorf("cannot expose Composer: %w", err)
		}
		ui.Success("Composer added to PATH")
		return nil
	},
}

func init() {
	pathCmd.Flags().BoolVar(&pathRemove, "remove", false, "Remove from PATH instead of adding")
}
