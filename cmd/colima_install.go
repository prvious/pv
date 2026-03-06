package cmd

import (
	"fmt"
	"net/http"
	"os"

	"github.com/prvious/pv/internal/colima"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var colimaInstallCmd = &cobra.Command{
	Use:   "colima:install",
	Short: "Install or update the Colima container runtime",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		client := &http.Client{}

		if err := ui.StepProgress("Installing Colima...", func(progress func(written, total int64)) (string, error) {
			if err := colima.Install(client, progress); err != nil {
				return "", fmt.Errorf("cannot install Colima: %w", err)
			}
			return "Colima installed", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(colimaInstallCmd)
}
