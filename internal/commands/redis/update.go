package redis

import (
	"fmt"
	"net/http"
	"time"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "redis:update",
	GroupID: "redis",
	Short:   "Re-download Redis (data dir untouched)",
	Example: `pv redis:update`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		if !r.IsInstalled() {
			return fmt.Errorf("redis is not installed")
		}

		wasRunning := false
		if st, err := r.LoadState(); err == nil && st.Wanted == r.WantedRunning {
			wasRunning = true
		}

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

		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress("Updating Redis...",
			func(progress func(written, total int64)) (string, error) {
				if err := r.UpdateProgress(client, progress); err != nil {
					return "", err
				}
				return "Updated Redis", nil
			}); err != nil {
			return err
		}

		if wasRunning {
			if err := r.SetWanted(r.WantedRunning); err != nil {
				return err
			}
		}

		ui.Success("Redis updated.")
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
