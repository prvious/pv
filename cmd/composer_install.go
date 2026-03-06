package cmd

import (
	"fmt"
	"net/http"
	"os"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var composerInstallCmd = &cobra.Command{
	Use:   "composer:install",
	Short: "Install or update Composer",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		client := &http.Client{}

		if err := ui.StepProgress("Installing Composer...", func(progress func(written, total int64)) (string, error) {
			if err := config.EnsureDirs(); err != nil {
				return "", err
			}

			vs, err := binaries.LoadVersions()
			if err != nil {
				return "", fmt.Errorf("cannot load version state: %w", err)
			}

			latest, err := binaries.FetchLatestVersion(client, binaries.Composer)
			if err != nil {
				return "", fmt.Errorf("cannot check Composer version: %w", err)
			}

			if err := binaries.InstallBinaryProgress(client, binaries.Composer, latest, progress); err != nil {
				return "", fmt.Errorf("cannot install Composer: %w", err)
			}

			vs.Set("composer", latest)
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			return "Composer installed", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(composerInstallCmd)
}
