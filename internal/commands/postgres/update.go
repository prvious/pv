package postgres

import (
	"fmt"
	"net/http"
	"time"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var updateCmd = &cobra.Command{
	Use:     "postgres:update <major>",
	GroupID: "postgres",
	Short:   "Re-download a PostgreSQL major (data dir untouched)",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := args[0]
		if !pg.IsInstalled(major) {
			return fmt.Errorf("postgres %s is not installed", major)
		}

		// Stop running process before swap.
		if err := pg.SetWanted(major, "stopped"); err != nil {
			return err
		}
		if server.IsRunning() {
			_ = server.SignalDaemon()
			time.Sleep(2 * time.Second)
		}

		client := &http.Client{Timeout: 5 * time.Minute}
		if err := ui.StepProgress(fmt.Sprintf("Updating PostgreSQL %s...", major),
			func(progress func(written, total int64)) (string, error) {
				if err := pg.UpdateProgress(client, major, progress); err != nil {
					return "", err
				}
				return fmt.Sprintf("Updated PostgreSQL %s", major), nil
			}); err != nil {
			return err
		}

		ui.Success(fmt.Sprintf("PostgreSQL %s updated.", major))
		if server.IsRunning() {
			return server.SignalDaemon()
		}
		return nil
	},
}
