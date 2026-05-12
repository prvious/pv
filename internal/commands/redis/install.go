package redis

import (
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "redis:install",
	GroupID: "redis",
	Short:   "Install (or re-install) Redis",
	Long:    "Downloads the Redis binary and registers it as wanted-running. No version arg — single-version service.",
	Example: `pv redis:install`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Already installed → idempotent: re-mark wanted=running and
		// signal the daemon. Same friendly contract postgres/mysql use.
		if r.IsInstalled() {
			if err := r.SetWanted(r.WantedRunning); err != nil {
				return err
			}
			ui.Success("Redis already installed — marked as wanted running.")
			return signalDaemon()
		}

		// Run the download/extract pipeline.
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}
		ui.Success("Redis installed.")
		return signalDaemon()
	},
}

// signalDaemon nudges the running pv daemon to reconcile, or no-ops with
// a friendly note if the daemon isn't up.
func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — redis will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
