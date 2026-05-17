package daemon

import (
	"fmt"

	internaldaemon "github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var disableCmd = &cobra.Command{
	Use:     "daemon:disable",
	GroupID: "daemon",
	Short:   "Disable the pv login daemon",
	RunE: func(cmd *cobra.Command, args []string) error {
		return ui.Step("Uninstalling pv daemon...", func() (string, error) {
			// Unload if loaded.
			if internaldaemon.IsLoaded() {
				if err := internaldaemon.Unload(); err != nil {
					return "", fmt.Errorf("cannot stop daemon: %w", err)
				}
			}

			if err := internaldaemon.Uninstall(); err != nil {
				return "", fmt.Errorf("cannot uninstall daemon: %w", err)
			}

			return "Daemon uninstalled", nil
		})
	},
}
