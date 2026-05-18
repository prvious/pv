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
		if daemon.IsLoaded() {
			return daemoncmds.RunRestart()
		}

		if !server.IsRunning() {
			return fmt.Errorf("pv is not running")
		}

		// Foreground mode — signal reconcile via SIGHUP.
		return ui.Step("Reconciling server...", func() (string, error) {
			if err := server.SignalDaemon(); err != nil {
				return "", fmt.Errorf("cannot signal server: %w", err)
			}
			return "Server reconciled", nil
		})
	},
}

func init() {
	rootCmd.AddCommand(restartCmd)
}
