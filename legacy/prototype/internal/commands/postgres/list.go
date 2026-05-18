package postgres

import (
	"fmt"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "postgres:list",
	GroupID: "postgres",
	Short:   "List installed PostgreSQL majors",
	RunE: func(cmd *cobra.Command, args []string) error {
		installed, err := pg.InstalledMajors()
		if err != nil {
			return err
		}
		if len(installed) == 0 {
			ui.Subtle("No PostgreSQL majors installed.")
			return nil
		}

		st, _ := pg.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		rows := [][]string{}
		for _, major := range installed {
			port, _ := pg.PortFor(major)
			version := "?"
			if vs != nil {
				if v := vs.Get("postgres-" + major); v != "" {
					version = v
				}
			}
			runState := "stopped"
			supKey := "postgres-" + major
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					runState = "running"
				}
			}
			wanted := st.Majors[major].Wanted
			projects := []string{}
			if reg != nil {
				for _, p := range reg.List() {
					if p.Services != nil && p.Services.Postgres == major {
						projects = append(projects, p.Name)
					}
				}
			}
			rows = append(rows, []string{
				major,
				version,
				fmt.Sprintf("%d", port),
				fmt.Sprintf("%s (%s)", runState, wanted),
				config.ServiceDataDir("postgres", major),
				fmt.Sprintf("%v", projects),
			})
		}

		ui.Table([]string{"MAJOR", "VERSION", "PORT", "STATUS", "DATA DIR", "PROJECTS"}, rows)
		return nil
	},
}
