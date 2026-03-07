package cmd

import (
	"fmt"

	daemoncmds "github.com/prvious/pv/internal/commands/daemon"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "restart",
	GroupID: "server",
	Short:   "Restart or reload the pv server",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Daemon mode — delegate to daemon:restart.
		if daemon.IsLoaded() {
			return daemoncmds.RunRestart()
		}

		// Foreground mode — reload config via admin API.
		if !server.IsRunning() {
			return fmt.Errorf("pv is not running")
		}

		return ui.Step("Reloading server configuration...", func() (string, error) {
			if err := server.ReconfigureServer(); err != nil {
				return "", fmt.Errorf("reconfigure failed: %w", err)
			}
			return "Configuration reloaded", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(restartCmd)
}
