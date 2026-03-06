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

var magoInstallCmd = &cobra.Command{
	Use:   "mago:install",
	Short: "Install or update Mago",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		client := &http.Client{}

		if err := ui.StepProgress("Installing Mago...", func(progress func(written, total int64)) (string, error) {
			if err := config.EnsureDirs(); err != nil {
				return "", err
			}

			vs, err := binaries.LoadVersions()
			if err != nil {
				return "", fmt.Errorf("cannot load version state: %w", err)
			}

			latest, err := binaries.FetchLatestVersion(client, binaries.Mago)
			if err != nil {
				return "", fmt.Errorf("cannot check Mago version: %w", err)
			}

			if err := binaries.InstallBinaryProgress(client, binaries.Mago, latest, progress); err != nil {
				return "", fmt.Errorf("cannot install Mago: %w", err)
			}

			vs.Set("mago", latest)
			if err := vs.Save(); err != nil {
				return "", fmt.Errorf("cannot save versions: %w", err)
			}

			return fmt.Sprintf("Mago %s installed", latest), nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(magoInstallCmd)
}
