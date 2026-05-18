package postgres

import (
	"fmt"
	"time"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var restartCmd = &cobra.Command{
	Use:     "postgres:restart [major]",
	GroupID: "postgres",
	Short:   "Stop and start a PostgreSQL major",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major, err := resolveMajor(args)
		if err != nil {
			return err
		}
		if err := pg.SetWanted(major, pg.WantedStopped); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return fmt.Errorf("signal daemon: %w", err)
			}
			if err := pg.WaitStopped(major, 30*time.Second); err != nil {
				return fmt.Errorf("waiting for postgres %s to stop: %w", major, err)
			}
		}
		if err := pg.SetWanted(major, pg.WantedRunning); err != nil {
			return err
		}
		if server.IsRunning() {
			if err := server.SignalDaemon(); err != nil {
				return err
			}
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s restarted.", major))
		return nil
	},
}
