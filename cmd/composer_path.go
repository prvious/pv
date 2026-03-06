package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var composerPathRemove bool

var composerPathCmd = &cobra.Command{
	Use:   "composer:path",
	Short: "Expose or remove Composer from PATH",
	RunE: func(cmd *cobra.Command, args []string) error {
		t := tools.Get("composer")

		if composerPathRemove {
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
	composerPathCmd.Flags().BoolVar(&composerPathRemove, "remove", false, "Remove from PATH instead of adding")
	rootCmd.AddCommand(composerPathCmd)
}
