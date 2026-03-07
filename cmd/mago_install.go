package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/spf13/cobra"
)

var magoInstallCmd = &cobra.Command{
	Use:   "mago:install",
	Short: "Install or update Mago",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Download.
		if err := magoDownloadCmd.RunE(magoDownloadCmd, nil); err != nil {
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

func init() {
	rootCmd.AddCommand(magoInstallCmd)
}
