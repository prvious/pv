package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var startCmd = &cobra.Command{
	Use:     "postgres:start [major]",
	GroupID: "postgres",
	Short:   "Mark a PostgreSQL major as wanted-running",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		if err := pg.SetWanted(major, pg.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s marked running.", major))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		ui.Subtle("daemon not running — will start on next `pv start`")
		return nil
	},
}
