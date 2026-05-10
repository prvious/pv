package redis

import (
	"fmt"

	"github.com/prvious/pv/internal/laravel"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

// init wires the per-project .env writer callback. Kept in the cobra
// layer rather than in internal/redis to avoid an import cycle:
// laravel imports redis (for EnvVars), so redis cannot import laravel.
func init() {
	r.EnvWriter = func(projectPath, projectName string, bound *registry.ProjectServices) error {
		return laravel.UpdateProjectEnvForRedis(projectPath, projectName, bound)
	}
}

var installCmd = &cobra.Command{
	Use:     "redis:install",
	GroupID: "redis",
	Short:   "Install (or re-install) Redis",
	Long:    "Downloads the Redis binary, registers it as wanted-running, and binds every linked Laravel project. No version arg — single-version service.",
	Example: `pv redis:install`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Already installed → idempotent: re-mark wanted=running, re-bind
		// linked projects (in case any were added since), and signal the
		// daemon. Same friendly contract postgres/mysql use.
		if r.IsInstalled() {
			if err := r.SetWanted(r.WantedRunning); err != nil {
				return err
			}
			if err := r.BindLinkedProjects(); err != nil {
				ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
			}
			ui.Success("Redis already installed — marked as wanted running.")
			return signalDaemon()
		}

		// Run the download/extract pipeline.
		if err := downloadCmd.RunE(downloadCmd, nil); err != nil {
			return err
		}
		if err := r.BindLinkedProjects(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
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
