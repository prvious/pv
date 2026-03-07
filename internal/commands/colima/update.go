package colima

import (
	"fmt"

	internalcolima "github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "colima:update",
	GroupID: "colima",
	Short:   "Update Colima to the latest version",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !internalcolima.IsInstalled() {
			ui.Success("Colima not installed (run: pv colima:install)")
			return nil
		}

		// Delegate download to :download.
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}

		// Re-expose if already on PATH.
		t := tools.MustGet("colima")
		if tools.IsExposed(t) {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Colima: %w", err)
			}
		}

		return nil
	},
}
