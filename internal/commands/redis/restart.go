package redis

import (
	"fmt"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "redis:restart",
	GroupID: "redis",
	Short:   "Stop and start Redis",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Phase 1: ask for stop, signal, wait for actual shutdown. Skipping
		// WaitStopped here would race with the supervisor's restart of the
		// next phase.
		if err := r.SetWanted(r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(10 * time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}
		// Phase 2: ask for running, signal once.
		if err := r.SetWanted(r.WantedRunning); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success("Redis restarted.")
		return nil
	},
}
