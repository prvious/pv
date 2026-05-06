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
		if err := pg.SetWanted(major, "stopped"); err != nil {
			return err
		}
		if server.IsRunning() {
			_ = server.SignalDaemon()
			time.Sleep(2 * time.Second)
		}
		if err := pg.SetWanted(major, "running"); err != nil {
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
