package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/daemon"
	"github.com/spf13/cobra"
)

var daemonCmd = &cobra.Command{
	Use:   "daemon",
	Short: "Manage the pv background daemon",
}

var daemonInstallCmd = &cobra.Command{
	Use:   "install",
	Short: "Install pv as a login daemon (starts on boot)",
	RunE: func(cmd *cobra.Command, args []string) error {
		cfg := daemon.DefaultPlistConfig()
		cfg.RunAtLoad = true

		if err := daemon.Install(cfg); err != nil {
			return fmt.Errorf("cannot install daemon: %w", err)
		}

		// Load the daemon so it starts immediately.
		if err := daemon.Load(); err != nil {
			return fmt.Errorf("cannot start daemon: %w", err)
		}

		fmt.Println("pv daemon installed (will start automatically on login)")
		return nil
	},
}

var daemonUninstallCmd = &cobra.Command{
	Use:   "uninstall",
	Short: "Uninstall the pv login daemon",
	RunE: func(cmd *cobra.Command, args []string) error {
		// Unload if loaded.
		if daemon.IsLoaded() {
			if err := daemon.Unload(); err != nil {
				return fmt.Errorf("cannot stop daemon: %w", err)
			}
		}

		if err := daemon.Uninstall(); err != nil {
			return fmt.Errorf("cannot uninstall daemon: %w", err)
		}

		fmt.Println("pv daemon uninstalled")
		return nil
	},
}

func init() {
	daemonCmd.AddCommand(daemonInstallCmd)
	daemonCmd.AddCommand(daemonUninstallCmd)
	rootCmd.AddCommand(daemonCmd)
}
