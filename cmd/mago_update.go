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

var magoUpdateCmd = &cobra.Command{
	Use:   "mago:update",
	Short: "Update Mago to the latest version",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		if err := config.EnsureDirs(); err != nil {
			return err
		}

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

		current := vs.Get("mago")

		if err := ui.StepProgress("Updating Mago...", func(progress func(written, total int64)) (string, error) {
			if err := binaries.InstallBinaryProgress(client, binaries.Mago, latest, progress); err != nil {
				return "", fmt.Errorf("cannot download Mago: %w", err)
			}

			vs.Set("mago", latest)
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			t := tools.Get("mago")
			if t != nil && tools.IsExposed(t) {
				if err := tools.Expose(t); err != nil {
					return "", fmt.Errorf("cannot expose Mago: %w", err)
				}
			}

			if current != "" {
				return fmt.Sprintf("Mago %s -> %s", current, latest), nil
			}
			return fmt.Sprintf("Mago %s", latest), nil
		}); err != nil {
			return err
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(magoUpdateCmd)
}
