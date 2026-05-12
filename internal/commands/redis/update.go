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
	Use:     "redis:update [version]",
	GroupID: "redis",
	Short:   "Re-download Redis (data dir untouched)",
	Example: `pv redis:update`,
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := resolveVersion(args)

		if !r.IsInstalled(version) {
			return fmt.Errorf("redis %s is not installed", version)
		}

		wasRunning := false
		if st, err := r.LoadState(); err == nil && st.Versions[version].Wanted == r.WantedRunning {
			wasRunning = true
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

		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress("Updating Redis...",
			func(progress func(written, total int64)) (string, error) {
				if err := r.UpdateProgress(client, version, progress); err != nil {
					return "", err
				}
				return "Updated Redis", nil
			}); err != nil {
			return err
		}

		if wasRunning {
			if err := r.SetWanted(version, r.WantedRunning); err != nil {
				return err
			}
		}

		ui.Success(fmt.Sprintf("Redis %s updated.", version))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
