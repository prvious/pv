package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:   "restart",
	Short: "Restart or reload the pv server",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Daemon mode — use launchctl kickstart for atomic restart.
		if daemon.IsLoaded() {
			if err := daemon.Restart(); err != nil {
				return fmt.Errorf("cannot restart daemon: %w", err)
			}
			fmt.Println("pv restarted")
			return nil
		}

		// Foreground mode — reload config via admin API.
		if !server.IsRunning() {
			return fmt.Errorf("pv is not running")
		}

		if err := server.ReconfigureServer(); err != nil {
			return fmt.Errorf("reconfigure failed: %w", err)
		}

		fmt.Println("Server configuration reloaded")
		return nil
	},
}

func init() {
	rootCmd.AddCommand(restartCmd)
}
