package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/tools"
	"github.com/spf13/cobra"
)

var composerInstallCmd = &cobra.Command{
	Use:   "composer:install",
	Short: "Install or update Composer",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		// Download.
		if err := composerDownloadCmd.RunE(composerDownloadCmd, nil); err != nil {
			return err
		}

		// Expose to PATH.
		t := tools.MustGet("composer")
		if t.AutoExpose {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Composer: %w", err)
			}
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(composerInstallCmd)
}
