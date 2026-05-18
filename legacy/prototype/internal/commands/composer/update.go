package composer

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "composer:update",
	GroupID: "composer",
	Short:   "Update Composer to the latest version",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Delegate download to :download (Composer always re-downloads).
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}

		// Re-expose if already on PATH.
		t := tools.MustGet("composer")
		if tools.IsExposed(t) {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Composer: %w", err)
			}
		}

		return nil
	},
}
