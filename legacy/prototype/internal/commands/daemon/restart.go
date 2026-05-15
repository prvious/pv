package daemon

import (
	"fmt"

	internaldaemon "github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "daemon:restart",
	GroupID: "daemon",
	Short:   "Restart the pv daemon",
	RunE: func(cmd *cobra.Command, args []string) error {
		if !internaldaemon.IsLoaded() {
			return fmt.Errorf("daemon is not running")
		}

		return ui.Step("Restarting pv daemon...", func() (string, error) {
			if err := internaldaemon.Restart(); err != nil {
				return "", fmt.Errorf("cannot restart daemon: %w", err)
			}
			return "Daemon restarted", nil
		})
	},
}
