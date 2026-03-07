package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var magoDownloadCmd = &cobra.Command{
	Use:   "mago:download",
	Short: "Download Mago to internal storage",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		if err := config.EnsureDirs(); err != nil {
			return err
		}

		return ui.StepProgress("Downloading Mago...", func(progress func(written, total int64)) (string, error) {
			vs, err := binaries.LoadVersions()
			if err != nil {
				return "", fmt.Errorf("cannot load version state: %w", err)
			}

			latest, err := binaries.FetchLatestVersion(client, binaries.Mago)
			if err != nil {
				return "", fmt.Errorf("cannot check Mago version: %w", err)
			}

			if err := binaries.InstallBinaryProgress(client, binaries.Mago, latest, progress); err != nil {
				return "", fmt.Errorf("cannot download Mago: %w", err)
			}

			vs.Set("mago", latest)
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			return fmt.Sprintf("Mago %s downloaded", latest), nil
		})
	},
}

func init() {
	rootCmd.AddCommand(magoDownloadCmd)
}
