package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var serviceCmd = &cobra.Command{
	Use:   "service",
	Short: "Manage the pv background service",
}

var serviceInstallCmd = &cobra.Command{
	Use:   "install",
	Short: "Install pv as a login service (starts on boot)",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		if err := ui.Step("Installing pv service...", func() (string, error) {
			cfg := daemon.DefaultPlistConfig()
			cfg.RunAtLoad = true

			if err := daemon.Install(cfg); err != nil {
				return "", fmt.Errorf("cannot install service: %w", err)
			}

			// Load the service so it starts immediately.
			if err := daemon.Load(); err != nil {
				return "", fmt.Errorf("cannot start service: %w", err)
			}

			return "Service installed (starts automatically on login)", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

var serviceUninstallCmd = &cobra.Command{
	Use:   "uninstall",
	Short: "Uninstall the pv login service",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		if err := ui.Step("Uninstalling pv service...", func() (string, error) {
			// Unload if loaded.
			if daemon.IsLoaded() {
				if err := daemon.Unload(); err != nil {
					return "", fmt.Errorf("cannot stop service: %w", err)
				}
			}

			if err := daemon.Uninstall(); err != nil {
				return "", fmt.Errorf("cannot uninstall service: %w", err)
			}

			return "Service uninstalled", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	serviceCmd.AddCommand(serviceInstallCmd)
	serviceCmd.AddCommand(serviceUninstallCmd)
	rootCmd.AddCommand(serviceCmd)
}
