package cmd

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var daemonEnableCmd = &cobra.Command{
	Use:   "daemon:enable",
	Short: "Enable pv as a login daemon (starts on boot)",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		if err := ui.Step("Installing pv daemon...", func() (string, error) {
			cfg := daemon.DefaultPlistConfig()
			cfg.RunAtLoad = true

			if err := daemon.Install(cfg); err != nil {
				return "", fmt.Errorf("cannot install daemon: %w", err)
			}

			// Load the daemon so it starts immediately.
			if err := daemon.Load(); err != nil {
				return "", fmt.Errorf("cannot start daemon: %w", err)
			}

			return "Daemon installed (starts automatically on login)", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

var daemonDisableCmd = &cobra.Command{
	Use:   "daemon:disable",
	Short: "Disable the pv login daemon",
	RunE: func(cmd *cobra.Command, args []string) error {
		fmt.Fprintln(os.Stderr)

		if err := ui.Step("Uninstalling pv daemon...", func() (string, error) {
			// Unload if loaded.
			if daemon.IsLoaded() {
				if err := daemon.Unload(); err != nil {
					return "", fmt.Errorf("cannot stop daemon: %w", err)
				}
			}

			if err := daemon.Uninstall(); err != nil {
				return "", fmt.Errorf("cannot uninstall daemon: %w", err)
			}

			return "Daemon uninstalled", nil
		}); err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(daemonEnableCmd)
	rootCmd.AddCommand(daemonDisableCmd)
}
