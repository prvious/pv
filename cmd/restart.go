package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:   "restart",
	Short: "Reload FrankenPHP configuration",
	RunE: func(cmd *cobra.Command, args []string) error {
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
