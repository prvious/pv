package redis

import (
	"fmt"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "redis:status [version]",
	GroupID: "redis",
	Short:   "Show Redis status",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)

		if !r.IsInstalled(version) {
			ui.Subtle(fmt.Sprintf("Redis %s is not installed.", version))
			return nil
		}
		status, _ := server.ReadDaemonStatus()
		if status != nil {
			key := "redis-" + version
			if s, ok := status.Supervised[key]; ok && s.Running {
				ui.Success(fmt.Sprintf("redis-%s: running on :%d (pid %d)", version, r.PortFor(version), s.PID))
				return nil
			}
		}
		ui.Subtle(fmt.Sprintf("redis-%s: stopped", version))
		return nil
	},
}
