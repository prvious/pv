package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:   "start",
	Short: "Start the pv server (DNS + FrankenPHP)",
	RunE: func(cmd *cobra.Command, args []string) error {
		if server.IsRunning() {
			return fmt.Errorf("pv is already running (PID file exists and process is alive)")
		}

		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}

		return server.Start(settings.TLD)
	},
}

func init() {
	rootCmd.AddCommand(startCmd)
}
