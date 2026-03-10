package daemon

import (
	"fmt"

	internaldaemon "github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var enableCmd = &cobra.Command{
	Use:     "daemon:enable",
	GroupID: "daemon",
	Short:   "Enable pv as a login daemon (starts on boot)",
	RunE: func(cmd *cobra.Command, args []string) error {
		if internaldaemon.IsLoaded() {
			// Already loaded — unload, regenerate plist (binary path, env vars, etc. may have changed), and reload.
			return ui.Step("Updating pv daemon...", func() (string, error) {
				if err := internaldaemon.Unload(); err != nil {
					return "", fmt.Errorf("cannot stop daemon for update: %w", err)
				}
				cfg := internaldaemon.DefaultPlistConfig()
				cfg.RunAtLoad = true
				if err := internaldaemon.Install(cfg); err != nil {
					return "", fmt.Errorf("cannot update daemon plist: %w", err)
				}
				if err := internaldaemon.Load(); err != nil {
					return "", fmt.Errorf("cannot restart daemon: %w", err)
				}
				return "Daemon updated and restarted", nil
			})
		}

		return ui.Step("Installing pv daemon...", func() (string, error) {
			cfg := internaldaemon.DefaultPlistConfig()
			cfg.RunAtLoad = true

			if err := internaldaemon.Install(cfg); err != nil {
				return "", fmt.Errorf("cannot install daemon: %w", err)
			}

			if err := internaldaemon.Load(); err != nil {
				return "", fmt.Errorf("cannot start daemon: %w", err)
			}

			return "Daemon installed (starts automatically on login)", nil
		})
	},
}
