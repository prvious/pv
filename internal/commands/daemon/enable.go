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
			// Already running — update plist in case binary path changed, then restart.
			return ui.Step("Updating pv daemon...", func() (string, error) {
				cfg := internaldaemon.DefaultPlistConfig()
				cfg.RunAtLoad = true
				if err := internaldaemon.Install(cfg); err != nil {
					return "", fmt.Errorf("cannot update daemon plist: %w", err)
				}
				if err := internaldaemon.Restart(); err != nil {
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
