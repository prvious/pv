package composer

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var downloadCmd = &cobra.Command{
	Use:     "composer:download",
	GroupID: "composer",
	Short:   "Download Composer to internal storage",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		if err := config.EnsureDirs(); err != nil {
			return err
		}

		return ui.StepProgress("Downloading Composer...", func(progress func(written, total int64)) (string, error) {
			vs, err := binaries.LoadVersions()
			if err != nil {
				return "", fmt.Errorf("cannot load version state: %w", err)
			}

			latest, err := binaries.FetchLatestVersion(client, binaries.Composer)
			if err != nil {
				return "", fmt.Errorf("cannot check Composer version: %w", err)
			}

			if err := binaries.InstallBinaryProgress(client, binaries.Composer, latest, progress); err != nil {
				return "", fmt.Errorf("cannot download Composer: %w", err)
			}

			vs.Set("composer", latest)
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			return "Composer downloaded", nil
		})
	},
}
