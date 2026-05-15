package mysql

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "mysql:list",
	GroupID: "mysql",
	Short:   "List installed MySQL versions",
	RunE: func(cmd *cobra.Command, args []string) error {
		installed, err := my.InstalledVersions()
		if err != nil {
			return err
		}
		if len(installed) == 0 {
			ui.Subtle("No MySQL versions installed.")
			return nil
		}

		st, _ := my.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		rows := [][]string{}
		for _, version := range installed {
			port, _ := my.PortFor(version)

			precise := "?"
			if vs != nil {
				if v := vs.Get("mysql-" + version); v != "" {
					precise = v
				}
			}

			runState := "stopped"
			supKey := "mysql-" + version
			if status != nil {
				if s, ok := status.Supervised[supKey]; ok && s.Running {
					runState = "running"
				}
			}
			wanted := st.Versions[version].Wanted

			projects := []string{}
			if reg != nil {
				for _, p := range reg.List() {
					if p.Services != nil && p.Services.MySQL == version {
						projects = append(projects, p.Name)
					}
				}
			}
			projectsCol := "—"
			if len(projects) > 0 {
				projectsCol = strings.Join(projects, ",")
			}

			rows = append(rows, []string{
				version,
				precise,
				fmt.Sprintf("%d", port),
				fmt.Sprintf("%s (%s)", runState, wanted),
				config.MysqlDataDir(version),
				projectsCol,
			})
		}

		ui.Table([]string{"VERSION", "PRECISE", "PORT", "STATUS", "DATA DIR", "LINKED PROJECTS"}, rows)
		return nil
	},
}
