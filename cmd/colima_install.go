package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/tools"
	"github.com/spf13/cobra"
)

var colimaInstallCmd = &cobra.Command{
	Use:   "colima:install",
	Short: "Install or update the Colima container runtime",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		// Download.
		if err := colimaDownloadCmd.RunE(colimaDownloadCmd, nil); err != nil {
			return err
		}

		// Expose (no-op for colima since AutoExpose=false).
		t := tools.MustGet("colima")
		if t.AutoExpose {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Colima: %w", err)
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(colimaInstallCmd)
}
