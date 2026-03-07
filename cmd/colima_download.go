package cmd

import (
	"fmt"
	"net/http"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var colimaDownloadCmd = &cobra.Command{
	Use:     "colima:download",
	GroupID: "colima",
	Short: "Download Colima to internal storage",
	RunE: func(cmd *cobra.Command, args []string) error {
		client := &http.Client{}

		return ui.StepProgress("Downloading Colima...", func(progress func(written, total int64)) (string, error) {
			if err := colima.Install(client, progress); err != nil {
				return "", fmt.Errorf("cannot download Colima: %w", err)
			}
			return "Colima downloaded", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(colimaDownloadCmd)
}
