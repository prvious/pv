package composer

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "composer:install",
	GroupID: "composer",
	Short:   "Install or update Composer",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Download.
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}

		// Expose to PATH.
		t := tools.MustGet("composer")
		if t.AutoExpose {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Composer: %w", err)
			}
		}

		return nil
	},
}
