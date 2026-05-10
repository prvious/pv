package redis

import (
	"fmt"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "redis:status",
	GroupID: "redis",
	Short:   "Show Redis status",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			ui.Subtle("Redis is not installed.")
			return nil
		}
		status, _ := server.ReadDaemonStatus()
		if status != nil {
			if s, ok := status.Supervised["redis"]; ok && s.Running {
				ui.Success(fmt.Sprintf("redis: running on :%d (pid %d)", r.PortFor(), s.PID))
				return nil
			}
		}
		ui.Subtle("redis: stopped")
		return nil
	},
}
