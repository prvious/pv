package redis

import (
	"fmt"

	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "redis:install [version]",
	GroupID: "redis",
	Short:   "Install (or re-install) Redis",
	Long:    "Downloads the Redis binary and registers it as wanted-running.",
	Example: `pv redis:install
  pv redis:install 8.6`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version, err := resolveVersion(args)
		if err != nil {
			return err
		}

		if r.IsInstalled(version) {
			if err := r.SetWanted(version, r.WantedRunning); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("Redis %s already installed — marked as wanted running.", version))
			return signalDaemon()
		}

		if err := downloadCmd.RunE(downloadCmd, []string{version}); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("Redis %s installed.", version))
		return signalDaemon()
	},
}

func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — redis will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
