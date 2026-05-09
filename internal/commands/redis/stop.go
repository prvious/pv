package redis

import (
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "redis:stop",
	GroupID: "redis",
	Short:   "Mark Redis as wanted-stopped",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		ui.Success("Redis marked stopped.")
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
