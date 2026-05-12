package redis

import (
	"fmt"
	"sort"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	r "github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var listCmd = &cobra.Command{
	Use:     "redis:list",
	GroupID: "redis",
	Short:   "Show installed Redis versions",
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		versions, err := r.InstalledVersions()
		if err != nil {
			return err
		}
		if len(versions) == 0 {
			ui.Subtle("No Redis versions installed.")
			return nil
		}
		sort.Strings(versions)

		st, _ := r.LoadState()
		vs, _ := binaries.LoadVersions()
		reg, _ := registry.Load()
		status, _ := server.ReadDaemonStatus()

		var rows [][]string
		for _, version := range versions {
			precise := "?"
			if vs != nil {
				if v := vs.Get("redis-" + version); v != "" {
					precise = v
				}
			}

			runState := "stopped"
			if status != nil {
				key := "redis-" + version
				if s, ok := status.Supervised[key]; ok && s.Running {
					runState = "running"
				}
			}

			wanted := st.Versions[version].Wanted
			if wanted == "" {
				wanted = "—"
			}

			projects := []string{}
			if reg != nil {
				for _, p := range reg.List() {
					if p.Services != nil && p.Services.Redis == version {
						projects = append(projects, p.Name)
					}
				}
			}
			projectsCol := "—"
			if len(projects) > 0 {
				projectsCol = strings.Join(projects, ",")
			}

			rows = append(rows, []string{
				precise,
				fmt.Sprintf("%d", r.PortFor(version)),
				fmt.Sprintf("%s (%s)", runState, wanted),
				config.RedisDataDirV(version),
				projectsCol,
			})
		}

		ui.Table([]string{"VERSION", "PORT", "STATUS", "DATA DIR", "LINKED PROJECTS"}, rows)
		return nil
	},
}
