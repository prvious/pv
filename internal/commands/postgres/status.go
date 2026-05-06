package postgres

import (
	"fmt"
	"os"

	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "postgres:status [major]",
	GroupID: "postgres",
	Short:   "Show PostgreSQL major status",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		var majors []string
		if len(args) > 0 {
			m, err := resolveMajor(args)
			if err != nil {
				return err
			}
			majors = []string{m}
		} else {
			ms, err := pg.InstalledMajors()
			if err != nil {
				return err
			}
			majors = ms
		}
		if len(majors) == 0 {
			ui.Subtle("No PostgreSQL majors installed.")
			return nil
		}

		status, _ := server.ReadDaemonStatus()
		for _, major := range majors {
			port, _ := pg.PortFor(major)
			supKey := "postgres-" + major
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					fmt.Fprintf(os.Stderr, "postgres %s: running on :%d (pid %d)\n", major, port, s.PID)
					continue
				}
			}
			fmt.Fprintf(os.Stderr, "postgres %s: stopped\n", major)
		}
		return nil
	},
}
