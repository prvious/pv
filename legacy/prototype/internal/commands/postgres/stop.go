package postgres

import (
	"fmt"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var stopCmd = &cobra.Command{
	Use:     "postgres:stop [major]",
	GroupID: "postgres",
	Short:   "Mark a PostgreSQL major as wanted-stopped",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		if err := pg.SetWanted(major, pg.WantedStopped); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s marked stopped.", major))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
