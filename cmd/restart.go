package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:   "restart",
	Short: "Restart or reload the pv server",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		// Daemon mode — delegate to daemon:restart.
		if daemon.IsLoaded() {
			return daemonRestartCmd.RunE(daemonRestartCmd, nil)
		}

		// Foreground mode — reload config via admin API.
		if !server.IsRunning() {
			ui.Subtle("pv is not running")
			fmt.Fprintln(os.Stderr)
			cmd.SilenceUsage = true
			return ui.ErrAlreadyPrinted
		}

		if err := ui.Step("Reloading server configuration...", func() (string, error) {
			if err := server.ReconfigureServer(); err != nil {
				return "", fmt.Errorf("reconfigure failed: %w", err)
			}
			return "Configuration reloaded", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(restartCmd)
}
