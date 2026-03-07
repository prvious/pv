package mago

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "mago:install",
	GroupID: "mago",
	Short: "Install or update Mago",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Download.
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}

		// Expose to PATH.
		t := tools.MustGet("mago")
		if t.AutoExpose {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Mago: %w", err)
			}
		}

		return nil
	},
}
