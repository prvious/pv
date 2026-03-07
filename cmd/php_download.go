package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var phpDownloadCmd = &cobra.Command{
	Use:     "php:download <version>",
	GroupID: "php",
	Short: "Download PHP + FrankenPHP to internal storage",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := args[0]
		if !validPHPVersion.MatchString(version) {
			return fmt.Errorf("invalid version format %q, use major.minor (e.g., 8.4)", version)
		}

		client := &http.Client{}
		return ui.StepProgress("Downloading PHP "+version+"...", func(progress func(written, total int64)) (string, error) {
			if err := phpenv.InstallProgress(client, version, progress); err != nil {
				return "", err
			}
			return fmt.Sprintf("PHP %s downloaded", version), nil
		})
	},
}

func init() {
	rootCmd.AddCommand(phpDownloadCmd)
}
