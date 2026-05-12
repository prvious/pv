package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

const defaultMajor = "18"

var installCmd = &cobra.Command{
	Use:     "postgres:install [major]",
	GroupID: "postgres",
	Short:   "Install (or re-install) a PostgreSQL major",
	Long:    "Downloads PostgreSQL binaries, runs initdb, and registers the major as wanted-running. Default major: 18.",
	Example: `# Install PostgreSQL 18 (default)
pv postgres:install

# Install PostgreSQL 17 alongside 18
pv postgres:install 17`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := defaultMajor
		if len(args) > 0 {
			major = args[0]
		}

		// If already on disk, refresh runtime state (conf overrides, hba,
		// socket dir) idempotently and mark wanted=running. This guards
		// against an /tmp socket dir reaped between boots and keeps
		// pv-managed conf in sync if the defaults changed across releases.
		if pg.IsInstalled(major) {
			if err := pg.EnsureRuntime(major); err != nil {
				return err
			}
			if err := pg.SetWanted(major, pg.WantedRunning); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("PostgreSQL %s already installed — marked as wanted running.", major))
			return signalDaemon()
		}

		// Run the download/install pipeline.
		if err := downloadCmd.RunE(downloadCmd, []string{major}); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s installed.", major))
		return signalDaemon()
	},
}

func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — postgres will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
