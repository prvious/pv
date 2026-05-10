package redis

import (
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "redis:start",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-running",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed (run `pv redis:install`).")
			return nil
		}
		if err := r.SetWanted(r.WantedRunning); err != nil {
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
