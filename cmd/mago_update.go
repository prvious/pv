package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var magoUpdateCmd = &cobra.Command{
	Use:     "mago:update",
	GroupID: "mago",
	Short: "Update Mago to the latest version",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		vs, err := binaries.LoadVersions()
		if err != nil {
			return fmt.Errorf("cannot load version state: %w", err)
		}

		latest, err := binaries.FetchLatestVersion(client, binaries.Mago)
		if err != nil {
			return fmt.Errorf("cannot check Mago version: %w", err)
		}

		if !binaries.NeedsUpdate(vs, binaries.Mago, latest) {
			ui.Success("Mago already up to date")
			return nil
		}

		// Delegate download to :download.
		if err := magoDownloadCmd.RunE(magoDownloadCmd, nil); err != nil {
			return err
		}

		// Re-expose if already on PATH.
		t := tools.MustGet("mago")
		if tools.IsExposed(t) {
			if err := tools.Expose(t); err != nil {
				return fmt.Errorf("cannot expose Mago: %w", err)
			}
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(magoUpdateCmd)
}
