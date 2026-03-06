package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var composerUpdateCmd = &cobra.Command{
	Use:   "composer:update",
	Short: "Update Composer to the latest version",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		if err := config.EnsureDirs(); err != nil {
			return err
		}

		// Composer always re-downloads (no version comparison).
		return ui.StepProgress("Updating Composer...", func(progress func(written, total int64)) (string, error) {
			vs, err := binaries.LoadVersions()
			if err != nil {
				return "", fmt.Errorf("cannot load version state: %w", err)
			}

			if err := binaries.InstallBinaryProgress(client, binaries.Composer, "latest", progress); err != nil {
				return "", fmt.Errorf("cannot download Composer: %w", err)
			}

			vs.Set("composer", "latest")
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			t := tools.Get("composer")
			if t != nil && tools.IsExposed(t) {
				if err := tools.Expose(t); err != nil {
					return "", fmt.Errorf("cannot expose Composer: %w", err)
				}
			}

			return "Composer updated", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(composerUpdateCmd)
}
