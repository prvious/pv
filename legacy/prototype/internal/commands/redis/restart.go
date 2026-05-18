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
	Use:     "redis:restart [version]",
	GroupID: "redis",
	Short:   "Stop and start Redis",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := resolveVersion(args)
		if err != nil {
			return err
		}

		if !r.IsInstalled(version) {
			return fmt.Errorf("redis %s is not installed", version)
		}

		if err := r.SetWanted(version, r.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := r.WaitStopped(version, 10*time.Second); err != nil {
				return fmt.Errorf("waiting for redis to stop: %w", err)
			}
		}
		if err := r.SetWanted(version, r.WantedRunning); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success(fmt.Sprintf("Redis %s restarted.", version))
		return nil
	},
}
