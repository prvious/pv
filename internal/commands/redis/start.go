package redis

import (
	"fmt"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "redis:start [version]",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-running",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)

		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed (run `pv redis:install`).", version))
			return nil
		}
		if err := r.SetWanted(version, r.WantedRunning); err != nil {
			return err
		}
		ui.Success("Redis marked running.")
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		ui.Subtle("daemon not running — will start on next `pv start`")
		return nil
	},
}
